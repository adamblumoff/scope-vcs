use super::{
    GeneratedIdSource, JobStore, acquire_aggregate_lock,
    cleanup_queue::queue_pending_source_blob_deletion_rows,
    entities,
    object_references::{delete_object_reference, insert_object_reference},
};
use sea_orm::{
    ActiveModelTrait, ColumnTrait, ConnectionTrait, DatabaseBackend, EntityTrait, IntoActiveModel,
    QueryFilter, QueryOrder, Statement, TransactionTrait,
};
use {
    crate::error::PostgresError,
    scope_domain::store::{GitPackSpan, validate_git_pack_layout, validate_git_pack_span_run},
};

#[derive(Clone, Debug)]
pub struct GitCompactionCandidate {
    pub repo_id: String,
    pub owner: String,
    pub name: String,
    pub predecessor: Option<GitPackSpan>,
    pub spans: Vec<GitPackSpan>,
}

impl JobStore {
    pub async fn git_compaction_candidate(
        &self,
        minimum_spans: u64,
    ) -> Result<Option<GitCompactionCandidate>, PostgresError> {
        if minimum_spans < 2 {
            return Err(PostgresError::internal_message(
                "Git compaction span threshold must be at least 2",
            ));
        }
        let minimum_spans = i64::try_from(minimum_spans).map_err(|_| {
            PostgresError::internal_message("Git compaction span threshold exceeds bigint")
        })?;
        let Some(candidate_row) = self
            .db
            .query_one(Statement::from_sql_and_values(
                DatabaseBackend::Postgres,
                r#"
                    WITH layout AS (
                        SELECT
                            repo_id,
                            geometric_tier,
                            lead(geometric_tier) OVER (
                                PARTITION BY repo_id ORDER BY first_sequence
                            ) AS next_tier,
                            count(*) OVER (PARTITION BY repo_id) AS span_count
                        FROM scope_git_segments
                    )
                    SELECT repo_id
                    FROM layout
                    WHERE span_count >= $1
                        AND geometric_tier = next_tier
                    GROUP BY repo_id
                    ORDER BY max(span_count) DESC, repo_id
                    LIMIT 1
                "#,
                [minimum_spans.into()],
            ))
            .await
            .map_err(PostgresError::internal)?
        else {
            return Ok(None);
        };
        let repo_id = candidate_row
            .try_get::<String>("", "repo_id")
            .map_err(PostgresError::internal)?;
        let repo = entities::repository::Entity::find_by_id(repo_id.clone())
            .one(self.db.as_ref())
            .await
            .map_err(PostgresError::internal)?
            .ok_or_else(|| PostgresError::internal_message("Git pack layout has no repository"))?;
        let spans = entities::git_pack_span::Entity::find()
            .filter(entities::git_pack_span::Column::RepoId.eq(repo_id.clone()))
            .order_by_asc(entities::git_pack_span::Column::FirstSequence)
            .all(self.db.as_ref())
            .await
            .map_err(PostgresError::internal)?
            .into_iter()
            .map(entities::git_pack_span::Model::try_into_domain)
            .collect::<Result<Vec<_>, _>>()?;
        validate_git_pack_layout(&spans)
            .map_err(|error| PostgresError::internal_message(error.to_string()))?;
        let Some(pair_start) = oldest_mergeable_pair_start(&spans, minimum_spans as usize) else {
            return Ok(None);
        };
        Ok(Some(GitCompactionCandidate {
            repo_id,
            owner: repo.owner_handle,
            name: repo.name,
            predecessor: pair_start
                .checked_sub(1)
                .and_then(|index| spans.get(index))
                .cloned(),
            spans: spans[pair_start..pair_start + 2].to_vec(),
        }))
    }

    pub async fn replace_git_pack_spans_with_compaction(
        &self,
        repo_id: &str,
        expected_spans: &[GitPackSpan],
        replacement: GitPackSpan,
        now_unix: u64,
        generated_ids: &dyn GeneratedIdSource,
    ) -> Result<bool, PostgresError> {
        validate_compaction_replacement(expected_spans, &replacement)?;
        let tx = self.db.begin().await.map_err(PostgresError::internal)?;
        acquire_aggregate_lock(&tx, "repository", repo_id).await?;
        let current_spans = entities::git_pack_span::Entity::find()
            .filter(entities::git_pack_span::Column::RepoId.eq(repo_id.to_string()))
            .order_by_asc(entities::git_pack_span::Column::FirstSequence)
            .all(&tx)
            .await
            .map_err(PostgresError::internal)?
            .into_iter()
            .map(entities::git_pack_span::Model::try_into_domain)
            .collect::<Result<Vec<_>, _>>()?;
        validate_git_pack_layout(&current_spans)
            .map_err(|error| PostgresError::internal_message(error.to_string()))?;

        let expected_first = expected_spans
            .first()
            .expect("replacement validation requires expected spans");
        let range_start = current_spans
            .iter()
            .position(|span| span.first_sequence == expected_first.first_sequence);
        let current_range = range_start.and_then(|start| {
            current_spans
                .get(start..start.checked_add(expected_spans.len())?)
                .map(|spans| (start, spans))
        });
        let Some((range_start, current_range)) = current_range else {
            queue_pending_source_blob_deletion_rows(
                &tx,
                [replacement.object],
                now_unix,
                generated_ids,
            )
            .await?;
            tx.commit().await.map_err(PostgresError::internal)?;
            return Ok(false);
        };
        if current_range != expected_spans {
            queue_pending_source_blob_deletion_rows(
                &tx,
                [replacement.object],
                now_unix,
                generated_ids,
            )
            .await?;
            tx.commit().await.map_err(PostgresError::internal)?;
            return Ok(false);
        }

        let range_end = range_start + expected_spans.len();
        let mut resulting_layout = Vec::with_capacity(current_spans.len() - 1);
        resulting_layout.extend(current_spans[..range_start].iter().cloned());
        resulting_layout.push(replacement.clone());
        resulting_layout.extend(current_spans[range_end..].iter().cloned());
        validate_git_pack_layout(&resulting_layout)
            .map_err(|error| PostgresError::internal_message(error.to_string()))?;

        for span in expected_spans {
            delete_object_reference(
                &tx,
                "git_segment",
                &format!("{repo_id}:{}", span.first_sequence),
            )
            .await?;
        }
        entities::git_pack_span::Entity::delete_many()
            .filter(entities::git_pack_span::Column::RepoId.eq(repo_id.to_string()))
            .filter(
                entities::git_pack_span::Column::FirstSequence.is_in(
                    expected_spans
                        .iter()
                        .map(|span| i64::try_from(span.first_sequence))
                        .collect::<Result<Vec<_>, _>>()
                        .map_err(|_| {
                            PostgresError::internal_message(
                                "Git pack span sequence exceeds database bigint",
                            )
                        })?,
                ),
            )
            .exec(&tx)
            .await
            .map_err(PostgresError::internal)?;
        entities::git_pack_span::Model::from_domain(repo_id, &replacement)?
            .into_active_model()
            .insert(&tx)
            .await
            .map_err(PostgresError::internal)?;
        insert_object_reference(
            &tx,
            "git_segment",
            &format!("{repo_id}:{}", replacement.first_sequence),
            &replacement.object,
        )
        .await?;

        let retired_objects = expected_spans
            .iter()
            .map(|span| span.object.clone())
            .filter(|object| object.content_ref != replacement.object.content_ref)
            .collect::<Vec<_>>();
        queue_pending_source_blob_deletion_rows(&tx, retired_objects, now_unix, generated_ids)
            .await?;
        tx.commit().await.map_err(PostgresError::internal)?;
        Ok(true)
    }
}

fn validate_compaction_replacement(
    expected_spans: &[GitPackSpan],
    replacement: &GitPackSpan,
) -> Result<(), PostgresError> {
    if expected_spans.len() != 2 {
        return Err(PostgresError::internal_message(
            "Git compaction requires exactly two expected pack spans",
        ));
    }
    validate_git_pack_span_run(expected_spans)
        .map_err(|error| PostgresError::internal_message(error.to_string()))?;
    let first = &expected_spans[0];
    let last = expected_spans
        .last()
        .expect("expected spans were checked as nonempty");
    if replacement.first_sequence != first.first_sequence
        || replacement.last_sequence != last.last_sequence
        || replacement.base_oid != first.base_oid
        || replacement.head_oid != last.head_oid
    {
        return Err(PostgresError::internal_message(
            "Git compaction replacement must cover exactly the selected pack spans",
        ));
    }
    if first.geometric_tier != last.geometric_tier {
        return Err(PostgresError::internal_message(
            "Git compaction requires adjacent pack spans from the same tier",
        ));
    }
    let expected_tier = replacement
        .expected_geometric_tier()
        .map_err(|error| PostgresError::internal_message(error.to_string()))?;
    if replacement.geometric_tier != expected_tier {
        return Err(PostgresError::internal_message(format!(
            "Git compaction replacement tier must be {expected_tier}"
        )));
    }
    Ok(())
}

fn oldest_mergeable_pair_start(spans: &[GitPackSpan], minimum_spans: usize) -> Option<usize> {
    if spans.len() < minimum_spans {
        return None;
    }
    spans.windows(2).position(|pair| {
        pair[0].geometric_tier == pair[1].geometric_tier
            && pair[0].last_sequence.checked_add(1) == Some(pair[1].first_sequence)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::{MetadataStore, TestDatabaseTarget, generated_ids::test_generated_id};
    use scope_domain::{content_ref::ContentRef, store::DEFAULT_GIT_FILE_MODE};
    use sea_orm::{ActiveModelTrait, IntoActiveModel};

    fn span(first_sequence: u64, last_sequence: u64, geometric_tier: u32) -> GitPackSpan {
        GitPackSpan {
            first_sequence,
            last_sequence,
            geometric_tier,
            base_oid: (first_sequence > 1).then(|| format!("head-{}", first_sequence - 1)),
            head_oid: format!("head-{last_sequence}"),
            object: scope_domain::store::SourceBlob {
                content_ref: ContentRef::git_segment_sha256(format!(
                    "pack-{first_sequence}-{last_sequence}"
                )),
                sha256: format!("pack-{first_sequence}-{last_sequence}"),
                git_oid: format!("head-{last_sequence}"),
                git_file_mode: DEFAULT_GIT_FILE_MODE.to_string(),
                size_bytes: 1,
            },
        }
    }

    #[test]
    fn candidate_pair_can_include_the_newest_persisted_span() {
        let spans = [span(1, 4, 2), span(5, 5, 0), span(6, 6, 0)];

        assert_eq!(oldest_mergeable_pair_start(&spans, 3), Some(1));
        assert_eq!(oldest_mergeable_pair_start(&spans[..2], 3), None);
    }

    #[test]
    fn candidate_chooses_the_oldest_equal_tier_pair() {
        let spans = [
            span(1, 4, 2),
            span(5, 6, 1),
            span(7, 8, 1),
            span(9, 9, 0),
            span(10, 10, 0),
        ];

        assert_eq!(oldest_mergeable_pair_start(&spans, 3), Some(1));
    }

    #[test]
    fn binary_frontier_advances_past_power_of_two_boundaries_with_a_fixed_limit() {
        let mut spans = Vec::new();
        for sequence in 1..=1_024 {
            assert!(spans.len() < 64, "push capacity deadlocked at {sequence}");
            spans.push(span(sequence, sequence, 0));
            if spans.len() >= 32 {
                let start = oldest_mergeable_pair_start(&spans, 32)
                    .expect("a full descending binary frontier has a mergeable pair");
                let replacement = span(
                    spans[start].first_sequence,
                    spans[start + 1].last_sequence,
                    spans[start].geometric_tier + 1,
                );
                spans.splice(start..start + 2, [replacement]);
            }
            validate_git_pack_layout(&spans).unwrap();
        }
        assert_eq!(spans.last().unwrap().last_sequence, 1_024);
    }

    #[test]
    fn replacement_must_preserve_selected_range_and_boundary_oids() {
        let expected = [span(1, 4, 2), span(5, 8, 2)];
        let valid = span(1, 8, 3);
        validate_compaction_replacement(&expected, &valid).unwrap();

        let wrong_head = GitPackSpan {
            head_oid: "different".to_string(),
            ..valid
        };
        assert!(validate_compaction_replacement(&expected, &wrong_head).is_err());
    }

    #[tokio::test]
    async fn compaction_replaces_an_interior_pair_and_preserves_both_sides() {
        let store =
            MetadataStore::connect_fresh_for_tests(&TestDatabaseTarget::required().unwrap())
                .unwrap();
        store
            .db
            .execute_unprepared(
                r#"
                    INSERT INTO scope_users (id, handle, email, email_verified)
                    VALUES ('user_compaction', 'compaction', 'compaction@scope.test', TRUE);
                    INSERT INTO scope_repositories (
                        id, owner_handle, name, owner_user_id, publication_state,
                        change_version, repo_config, policy
                    ) VALUES (
                        'repo_compaction', 'compaction', 'repo', 'user_compaction', 'Ready',
                        4,
                        '{"kind":"scope.repo-config","version":1,"visibility":{"default":"private","rules":[]}}'::jsonb,
                        '{"default_visibility":"Private","rules":[]}'::jsonb
                    );
                    INSERT INTO scope_git_heads (
                        repo_id, head_oid, push_sequence, change_version,
                        manifest_object_key, manifest_sha256, manifest_size_bytes
                    ) VALUES (
                        'repo_compaction', 'head-4', 4, 4,
                        '{"GitManifestSha256":"manifest-4"}', 'manifest-4', 10
                    );
                "#,
            )
            .await
            .unwrap();
        let initial = [span(1, 2, 1), span(3, 3, 0), span(4, 4, 0)];
        for span in initial {
            entities::git_pack_span::Model::from_domain("repo_compaction", &span)
                .unwrap()
                .into_active_model()
                .insert(store.db.as_ref())
                .await
                .unwrap();
            insert_object_reference(
                store.db.as_ref(),
                "git_segment",
                &format!("repo_compaction:{}", span.first_sequence),
                &span.object,
            )
            .await
            .unwrap();
        }

        let candidate = store
            .jobs()
            .git_compaction_candidate(3)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            candidate
                .spans
                .iter()
                .map(|span| (span.first_sequence, span.last_sequence))
                .collect::<Vec<_>>(),
            [(3, 3), (4, 4)]
        );
        assert_eq!(
            candidate
                .predecessor
                .as_ref()
                .map(|span| (span.first_sequence, span.last_sequence)),
            Some((1, 2))
        );

        let appended = span(5, 5, 0);
        entities::git_pack_span::Model::from_domain("repo_compaction", &appended)
            .unwrap()
            .into_active_model()
            .insert(store.db.as_ref())
            .await
            .unwrap();
        insert_object_reference(
            store.db.as_ref(),
            "git_segment",
            "repo_compaction:5",
            &appended.object,
        )
        .await
        .unwrap();
        store
            .db
            .execute_unprepared(
                "UPDATE scope_git_heads
                 SET head_oid = 'head-5', push_sequence = 5, change_version = 5
                 WHERE repo_id = 'repo_compaction'",
            )
            .await
            .unwrap();

        let replacement = span(3, 4, 1);
        let applied = store
            .jobs()
            .replace_git_pack_spans_with_compaction(
                "repo_compaction",
                &candidate.spans,
                replacement,
                10,
                &test_generated_id,
            )
            .await
            .unwrap();
        assert!(applied);

        let layout = entities::git_pack_span::Entity::find()
            .filter(entities::git_pack_span::Column::RepoId.eq("repo_compaction"))
            .order_by_asc(entities::git_pack_span::Column::FirstSequence)
            .all(store.db.as_ref())
            .await
            .unwrap()
            .into_iter()
            .map(entities::git_pack_span::Model::try_into_domain)
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
        assert_eq!(
            layout
                .iter()
                .map(|span| (span.first_sequence, span.last_sequence))
                .collect::<Vec<_>>(),
            [(1, 2), (3, 4), (5, 5)]
        );
        let head = entities::git_head::Entity::find_by_id("repo_compaction")
            .one(store.db.as_ref())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(head.push_sequence, 5);
        assert_eq!(head.head_oid, "head-5");
    }
}
