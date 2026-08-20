use super::{
    GeneratedIdKind, GeneratedIdSource, JobStore, acquire_aggregate_lock,
    cleanup_queue::queue_pending_source_blob_deletion_rows,
    entities,
    generated_ids::generate_id,
    object_references::{delete_object_reference, insert_object_reference},
};
use sea_orm::{
    ActiveModelTrait, ColumnTrait, ConnectionTrait, DatabaseBackend, EntityTrait, FromQueryResult,
    IntoActiveModel, QueryFilter, QueryOrder, Statement, TransactionTrait,
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

#[derive(Clone, Debug)]
pub struct GitCompactionClaim {
    pub target_sequence: u64,
    pub attempts: u32,
    pub queue_delay_ms: u64,
    pub candidate: Option<GitCompactionCandidate>,
    repo_id: String,
    lease_generation: String,
}

const MAX_COMPACTION_RETRY_SECONDS: i64 = 3_600;

pub(super) async fn schedule_git_compaction<C>(
    conn: &C,
    repo_id: &str,
    target_sequence: u64,
    now_unix: u64,
) -> Result<(), PostgresError>
where
    C: ConnectionTrait,
{
    let target_sequence = i64::try_from(target_sequence).map_err(|_| {
        PostgresError::internal_message("Git compaction target exceeds database bigint")
    })?;
    let now = i64::try_from(now_unix).map_err(|_| {
        PostgresError::internal_message("Git compaction schedule time exceeds database bigint")
    })?;
    conn.execute(Statement::from_sql_and_values(
        DatabaseBackend::Postgres,
        r#"
            INSERT INTO scope_git_compaction_jobs (
                repo_id, target_sequence, attempts, next_run_at_unix,
                lease_generation, lease_owner, lease_expires_at_unix,
                last_error, created_at_unix, updated_at_unix
            ) VALUES ($1, $2, 0, $3, NULL, NULL, NULL, NULL, $3, $3)
            ON CONFLICT (repo_id) DO UPDATE
            SET target_sequence = GREATEST(
                    scope_git_compaction_jobs.target_sequence,
                    EXCLUDED.target_sequence
                ),
                updated_at_unix = GREATEST(
                    scope_git_compaction_jobs.updated_at_unix,
                    EXCLUDED.updated_at_unix
                )
        "#,
        [repo_id.into(), target_sequence.into(), now.into()],
    ))
    .await
    .map_err(PostgresError::internal)?;
    Ok(())
}

impl JobStore {
    pub async fn claim_git_compaction(
        &self,
        worker_id: &str,
        minimum_spans: u64,
        now_unix: u64,
        lease_seconds: u64,
        generated_ids: &dyn GeneratedIdSource,
    ) -> Result<Option<GitCompactionClaim>, PostgresError> {
        if worker_id.trim().is_empty() {
            return Err(PostgresError::internal_message(
                "Git compaction worker identity is empty",
            ));
        }
        if minimum_spans < 2 {
            return Err(PostgresError::internal_message(
                "Git compaction span threshold must be at least 2",
            ));
        }
        if lease_seconds == 0 {
            return Err(PostgresError::internal_message(
                "Git compaction lease must be greater than zero",
            ));
        }
        let now = i64::try_from(now_unix).map_err(|_| {
            PostgresError::internal_message("Git compaction claim time exceeds database bigint")
        })?;
        let lease_seconds = i64::try_from(lease_seconds).map_err(|_| {
            PostgresError::internal_message("Git compaction lease exceeds database bigint")
        })?;
        let lease_expires = now.checked_add(lease_seconds).ok_or_else(|| {
            PostgresError::internal_message("Git compaction lease expiry exceeds database bigint")
        })?;
        let lease_generation = generate_id(generated_ids, GeneratedIdKind::CleanupGeneration)?;
        let tx = self.db.begin().await.map_err(PostgresError::internal)?;
        let Some(job) =
            entities::git_compaction_job::Model::find_by_statement(Statement::from_sql_and_values(
                DatabaseBackend::Postgres,
                r#"
                    UPDATE scope_git_compaction_jobs AS job
                    SET lease_generation = $4,
                        lease_owner = $3,
                        lease_expires_at_unix = $2,
                        updated_at_unix = $1
                    FROM (
                        SELECT repo_id
                        FROM scope_git_compaction_jobs
                        WHERE next_run_at_unix <= $1
                            AND (
                                lease_expires_at_unix IS NULL OR
                                lease_expires_at_unix <= $1
                            )
                        ORDER BY next_run_at_unix, updated_at_unix, repo_id
                        FOR UPDATE SKIP LOCKED
                        LIMIT 1
                    ) AS claimed
                    WHERE job.repo_id = claimed.repo_id
                    RETURNING job.*
                "#,
                [
                    now.into(),
                    lease_expires.into(),
                    worker_id.into(),
                    lease_generation.clone().into(),
                ],
            ))
            .one(&tx)
            .await
            .map_err(PostgresError::internal)?
        else {
            tx.commit().await.map_err(PostgresError::internal)?;
            return Ok(None);
        };
        let repo = entities::repository::Entity::find_by_id(job.repo_id.clone())
            .one(&tx)
            .await
            .map_err(PostgresError::internal)?
            .ok_or_else(|| {
                PostgresError::internal_message("Git compaction job has no repository")
            })?;
        let spans = entities::git_pack_span::Entity::find()
            .filter(entities::git_pack_span::Column::RepoId.eq(job.repo_id.clone()))
            .order_by_asc(entities::git_pack_span::Column::FirstSequence)
            .all(&tx)
            .await
            .map_err(PostgresError::internal)?
            .into_iter()
            .map(entities::git_pack_span::Model::try_into_domain)
            .collect::<Result<Vec<_>, _>>()?;
        validate_git_pack_layout(&spans)
            .map_err(|error| PostgresError::internal_message(error.to_string()))?;
        let candidate =
            oldest_mergeable_pair_start(&spans, minimum_spans as usize).map(|pair_start| {
                GitCompactionCandidate {
                    repo_id: job.repo_id.clone(),
                    owner: repo.owner_handle,
                    name: repo.name,
                    predecessor: pair_start
                        .checked_sub(1)
                        .and_then(|index| spans.get(index))
                        .cloned(),
                    spans: spans[pair_start..pair_start + 2].to_vec(),
                }
            });
        let target_sequence = u64::try_from(job.target_sequence).map_err(|_| {
            PostgresError::internal_message("Git compaction target sequence is negative")
        })?;
        let attempts = u32::try_from(job.attempts).map_err(|_| {
            PostgresError::internal_message("Git compaction attempt count is invalid")
        })?;
        let due_at_unix = u64::try_from(job.next_run_at_unix)
            .map_err(|_| PostgresError::internal_message("Git compaction due time is negative"))?;
        tx.commit().await.map_err(PostgresError::internal)?;
        Ok(Some(GitCompactionClaim {
            target_sequence,
            attempts,
            queue_delay_ms: now_unix.saturating_sub(due_at_unix).saturating_mul(1_000),
            candidate,
            repo_id: job.repo_id,
            lease_generation,
        }))
    }

    pub async fn complete_git_compaction_claim(
        &self,
        claim: &GitCompactionClaim,
        now_unix: u64,
    ) -> Result<(), PostgresError> {
        let now = compaction_time(now_unix)?;
        let tx = self.db.begin().await.map_err(PostgresError::internal)?;
        tx.execute(Statement::from_sql_and_values(
            DatabaseBackend::Postgres,
            r#"
                UPDATE scope_git_compaction_jobs
                SET lease_generation = NULL,
                    lease_owner = NULL,
                    lease_expires_at_unix = NULL,
                    next_run_at_unix = $3,
                    attempts = 0,
                    last_error = NULL,
                    updated_at_unix = GREATEST(updated_at_unix, $3)
                WHERE repo_id = $1
                    AND lease_generation = $2
                    AND target_sequence > $4
            "#,
            [
                claim.repo_id.clone().into(),
                claim.lease_generation.clone().into(),
                now.into(),
                i64::try_from(claim.target_sequence)
                    .map_err(|_| {
                        PostgresError::internal_message(
                            "Git compaction target exceeds database bigint",
                        )
                    })?
                    .into(),
            ],
        ))
        .await
        .map_err(PostgresError::internal)?;
        tx.execute(Statement::from_sql_and_values(
            DatabaseBackend::Postgres,
            "DELETE FROM scope_git_compaction_jobs
             WHERE repo_id = $1 AND lease_generation = $2 AND target_sequence <= $3",
            [
                claim.repo_id.clone().into(),
                claim.lease_generation.clone().into(),
                i64::try_from(claim.target_sequence)
                    .map_err(|_| {
                        PostgresError::internal_message(
                            "Git compaction target exceeds database bigint",
                        )
                    })?
                    .into(),
            ],
        ))
        .await
        .map_err(PostgresError::internal)?;
        tx.commit().await.map_err(PostgresError::internal)
    }

    pub async fn renew_git_compaction_claim(
        &self,
        claim: &GitCompactionClaim,
        now_unix: u64,
        lease_seconds: u64,
    ) -> Result<bool, PostgresError> {
        if lease_seconds == 0 {
            return Err(PostgresError::internal_message(
                "Git compaction lease must be greater than zero",
            ));
        }
        let now = compaction_time(now_unix)?;
        let lease_seconds = i64::try_from(lease_seconds).map_err(|_| {
            PostgresError::internal_message("Git compaction lease exceeds database bigint")
        })?;
        let lease_expires = now.checked_add(lease_seconds).ok_or_else(|| {
            PostgresError::internal_message("Git compaction lease expiry exceeds database bigint")
        })?;
        let result = self
            .db
            .execute(Statement::from_sql_and_values(
                DatabaseBackend::Postgres,
                r#"
                    UPDATE scope_git_compaction_jobs
                    SET lease_expires_at_unix = $3,
                        updated_at_unix = GREATEST(updated_at_unix, $2)
                    WHERE repo_id = $1
                        AND lease_generation = $4
                        AND lease_expires_at_unix > $2
                "#,
                [
                    claim.repo_id.clone().into(),
                    now.into(),
                    lease_expires.into(),
                    claim.lease_generation.clone().into(),
                ],
            ))
            .await
            .map_err(PostgresError::internal)?;
        Ok(result.rows_affected() == 1)
    }

    pub async fn continue_git_compaction_claim(
        &self,
        claim: &GitCompactionClaim,
        now_unix: u64,
    ) -> Result<(), PostgresError> {
        let now = compaction_time(now_unix)?;
        self.db
            .execute(Statement::from_sql_and_values(
                DatabaseBackend::Postgres,
                r#"
                    UPDATE scope_git_compaction_jobs
                    SET lease_generation = NULL,
                        lease_owner = NULL,
                        lease_expires_at_unix = NULL,
                        next_run_at_unix = $3,
                        attempts = 0,
                        last_error = NULL,
                        updated_at_unix = GREATEST(updated_at_unix, $3)
                    WHERE repo_id = $1 AND lease_generation = $2
                "#,
                [
                    claim.repo_id.clone().into(),
                    claim.lease_generation.clone().into(),
                    now.into(),
                ],
            ))
            .await
            .map_err(PostgresError::internal)?;
        Ok(())
    }

    pub async fn fail_git_compaction_claim(
        &self,
        claim: &GitCompactionClaim,
        error: &str,
        now_unix: u64,
    ) -> Result<(), PostgresError> {
        let now = compaction_time(now_unix)?;
        let error = bounded_compaction_error(error);
        self.db
            .execute(Statement::from_sql_and_values(
                DatabaseBackend::Postgres,
                r#"
                    UPDATE scope_git_compaction_jobs
                    SET lease_generation = NULL,
                        lease_owner = NULL,
                        lease_expires_at_unix = NULL,
                        attempts = attempts + 1,
                        next_run_at_unix = $3 + LEAST(
                            $5,
                            5 * (1::bigint << LEAST(attempts, 9))
                        ),
                        last_error = $4,
                        updated_at_unix = GREATEST(updated_at_unix, $3)
                    WHERE repo_id = $1 AND lease_generation = $2
                "#,
                [
                    claim.repo_id.clone().into(),
                    claim.lease_generation.clone().into(),
                    now.into(),
                    error.into(),
                    MAX_COMPACTION_RETRY_SECONDS.into(),
                ],
            ))
            .await
            .map_err(PostgresError::internal)?;
        Ok(())
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

fn compaction_time(now_unix: u64) -> Result<i64, PostgresError> {
    i64::try_from(now_unix)
        .map_err(|_| PostgresError::internal_message("Git compaction time exceeds database bigint"))
}

fn bounded_compaction_error(error: &str) -> String {
    let mut bounded = error.trim().chars().take(2_000).collect::<String>();
    if bounded.is_empty() {
        bounded = "Git compaction failed without a diagnostic".to_string();
    }
    bounded
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

    async fn seed_scheduled_repo(store: &MetadataStore) {
        store
            .db
            .execute_unprepared(
                r#"
                    INSERT INTO scope_users (id, handle, email, email_verified)
                    VALUES ('scheduler_user', 'scheduler-user', 'scheduler@scope.test', TRUE);
                    INSERT INTO scope_repositories (
                        id, owner_handle, name, owner_user_id, publication_state,
                        change_version, repo_config, policy
                    ) VALUES (
                        'scheduler/repo', 'scheduler-user', 'repo', 'scheduler_user', 'Ready',
                        1,
                        '{"kind":"scope.repo-config","version":1,"visibility":{"default":"private","rules":[]}}'::jsonb,
                        '{"default_visibility":"Private","rules":[]}'::jsonb
                    );
                "#,
            )
            .await
            .unwrap();
        entities::git_pack_span::Model::from_domain("scheduler/repo", &span(1, 1, 0))
            .unwrap()
            .into_active_model()
            .insert(store.db.as_ref())
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn lease_reclaim_rejects_the_old_workers_completion() {
        let store =
            MetadataStore::connect_fresh_for_tests(&TestDatabaseTarget::required().unwrap())
                .unwrap();
        seed_scheduled_repo(&store).await;
        schedule_git_compaction(store.db.as_ref(), "scheduler/repo", 1, 10)
            .await
            .unwrap();

        let first = store
            .jobs()
            .claim_git_compaction("worker-a", 2, 10, 10, &test_generated_id)
            .await
            .unwrap()
            .unwrap();
        assert!(
            store
                .jobs()
                .claim_git_compaction("worker-b", 2, 10, 10, &test_generated_id)
                .await
                .unwrap()
                .is_none()
        );
        let reclaimed = store
            .jobs()
            .claim_git_compaction("worker-b", 2, 21, 10, &test_generated_id)
            .await
            .unwrap()
            .unwrap();

        store
            .jobs()
            .complete_git_compaction_claim(&first, 22)
            .await
            .unwrap();
        let job = entities::git_compaction_job::Entity::find_by_id("scheduler/repo")
            .one(store.db.as_ref())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(job.lease_owner.as_deref(), Some("worker-b"));

        store
            .jobs()
            .complete_git_compaction_claim(&reclaimed, 22)
            .await
            .unwrap();
        assert!(
            entities::git_compaction_job::Entity::find_by_id("scheduler/repo")
                .one(store.db.as_ref())
                .await
                .unwrap()
                .is_none()
        );
    }

    #[tokio::test]
    async fn lease_renewal_prevents_reclaim_until_the_extended_expiry() {
        let store =
            MetadataStore::connect_fresh_for_tests(&TestDatabaseTarget::required().unwrap())
                .unwrap();
        seed_scheduled_repo(&store).await;
        schedule_git_compaction(store.db.as_ref(), "scheduler/repo", 1, 10)
            .await
            .unwrap();

        let claim = store
            .jobs()
            .claim_git_compaction("worker-a", 2, 10, 10, &test_generated_id)
            .await
            .unwrap()
            .unwrap();
        assert!(
            store
                .jobs()
                .renew_git_compaction_claim(&claim, 15, 10)
                .await
                .unwrap()
        );
        assert!(
            store
                .jobs()
                .claim_git_compaction("worker-b", 2, 21, 10, &test_generated_id)
                .await
                .unwrap()
                .is_none()
        );
        assert!(
            store
                .jobs()
                .claim_git_compaction("worker-b", 2, 25, 10, &test_generated_id)
                .await
                .unwrap()
                .is_some()
        );
        assert!(
            !store
                .jobs()
                .renew_git_compaction_claim(&claim, 26, 10)
                .await
                .unwrap()
        );
    }

    #[tokio::test]
    async fn push_scheduled_during_a_claim_survives_completion() {
        let store =
            MetadataStore::connect_fresh_for_tests(&TestDatabaseTarget::required().unwrap())
                .unwrap();
        seed_scheduled_repo(&store).await;
        schedule_git_compaction(store.db.as_ref(), "scheduler/repo", 1, 10)
            .await
            .unwrap();
        let first = store
            .jobs()
            .claim_git_compaction("worker-a", 2, 10, 30, &test_generated_id)
            .await
            .unwrap()
            .unwrap();

        schedule_git_compaction(store.db.as_ref(), "scheduler/repo", 2, 11)
            .await
            .unwrap();
        store
            .jobs()
            .complete_git_compaction_claim(&first, 12)
            .await
            .unwrap();

        let next = store
            .jobs()
            .claim_git_compaction("worker-b", 2, 12, 30, &test_generated_id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(next.target_sequence, 2);
    }

    #[tokio::test]
    async fn new_push_does_not_bypass_a_failed_compactions_backoff() {
        let store =
            MetadataStore::connect_fresh_for_tests(&TestDatabaseTarget::required().unwrap())
                .unwrap();
        seed_scheduled_repo(&store).await;
        schedule_git_compaction(store.db.as_ref(), "scheduler/repo", 1, 10)
            .await
            .unwrap();
        let failed = store
            .jobs()
            .claim_git_compaction("worker-a", 2, 10, 30, &test_generated_id)
            .await
            .unwrap()
            .unwrap();
        store
            .jobs()
            .fail_git_compaction_claim(&failed, "bounded failure", 10)
            .await
            .unwrap();

        schedule_git_compaction(store.db.as_ref(), "scheduler/repo", 2, 11)
            .await
            .unwrap();
        assert!(
            store
                .jobs()
                .claim_git_compaction("worker-b", 2, 14, 30, &test_generated_id)
                .await
                .unwrap()
                .is_none()
        );
        let retry = store
            .jobs()
            .claim_git_compaction("worker-b", 2, 15, 30, &test_generated_id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(retry.target_sequence, 2);
        assert_eq!(retry.attempts, 1);
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

        schedule_git_compaction(store.db.as_ref(), "repo_compaction", 4, 10)
            .await
            .unwrap();
        let claim = store
            .jobs()
            .claim_git_compaction("worker-a", 3, 10, 60, &test_generated_id)
            .await
            .unwrap()
            .unwrap();
        let candidate = claim.candidate.unwrap();
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
