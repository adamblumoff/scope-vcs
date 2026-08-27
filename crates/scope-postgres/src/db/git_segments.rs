use super::{RepositoryStore, content_fences, entities};
use crate::error::PostgresError;
use scope_domain::repository::git::{GitPackSpan, GitSegmentRef, GitSegmentUpload};
use sea_orm::{
    ColumnTrait, ConnectionTrait, DatabaseBackend, EntityTrait, QueryFilter, QueryOrder,
    QuerySelect, Statement,
};
use sqlx::PgConnection;
use std::collections::BTreeMap;

pub struct RepositoryGitWriteLease {
    connection: Option<PgConnection>,
    key: i64,
}

impl RepositoryGitWriteLease {
    pub async fn release(mut self) {
        let Some(mut connection) = self.connection.take() else {
            return;
        };
        if let Err(error) = sqlx::query("SELECT pg_advisory_unlock($1)")
            .bind(self.key)
            .execute(&mut connection)
            .await
        {
            tracing::warn!(
                error = %error,
                "failed to release repository Git write lease; dropping its Postgres session"
            );
        }
    }
}

impl Drop for RepositoryGitWriteLease {
    fn drop(&mut self) {
        self.connection.take();
    }
}

impl RepositoryStore {
    pub async fn acquire_git_write_lease(
        &self,
        repo_id: &str,
    ) -> Result<RepositoryGitWriteLease, PostgresError> {
        if repo_id.trim().is_empty() {
            return Err(PostgresError::internal_message(
                "repository Git write lease requires a repository id",
            ));
        }
        let schema = content_fences::current_schema(self.db.as_ref()).await?;
        let mut connection = content_fences::dedicated_fence_connection(
            self.postgres_database_url.as_deref(),
            &schema,
        )
        .await?;
        let key = sea_orm::sqlx::postgres::PgAdvisoryLock::new(format!(
            "scope:git-write:{schema}:{repo_id}"
        ))
        .key()
        .as_bigint()
        .expect("string-derived advisory locks use one bigint key");
        sqlx::query("SELECT pg_advisory_lock($1)")
            .bind(key)
            .execute(&mut connection)
            .await
            .map_err(PostgresError::internal)?;
        Ok(RepositoryGitWriteLease {
            connection: Some(connection),
            key,
        })
    }

    pub async fn begin_git_segment_upload(
        &self,
        repo_id: &str,
        segment_id: &str,
        object_key: &str,
        encoding_version: u32,
        now_unix: u64,
    ) -> Result<(), PostgresError> {
        require_text(repo_id, "Git segment repository id")?;
        require_text(segment_id, "Git segment id")?;
        require_text(object_key, "Git segment object key")?;
        if encoding_version == 0 {
            return Err(PostgresError::internal_message(
                "Git segment encoding version must be positive",
            ));
        }
        self.db
            .execute(Statement::from_sql_and_values(
                DatabaseBackend::Postgres,
                "INSERT INTO scope_git_segment_uploads (
                    segment_id, repo_id, object_key, state, sha256,
                    plaintext_bytes, encrypted_bytes, encoding_version,
                    created_at_unix, updated_at_unix
                 )
                 SELECT $1, $2, $3, 'uploading', NULL, NULL, NULL, $4, $5, $5
                 FROM scope_repositories WHERE id = $2",
                [
                    segment_id.into(),
                    repo_id.into(),
                    object_key.into(),
                    i32::try_from(encoding_version)
                        .map_err(|_| {
                            PostgresError::internal_message(
                                "Git segment encoding version exceeds database integer",
                            )
                        })?
                        .into(),
                    timestamp(now_unix, "Git segment upload creation time")?.into(),
                ],
            ))
            .await
            .map_err(PostgresError::internal)
            .and_then(|result| {
                if result.rows_affected() == 1 {
                    Ok(())
                } else {
                    Err(PostgresError::not_found(format!(
                        "repository {repo_id} not found"
                    )))
                }
            })
    }

    pub async fn mark_git_segment_upload_ready(
        &self,
        segment: &GitSegmentRef,
        encrypted_bytes: u64,
        now_unix: u64,
    ) -> Result<(), PostgresError> {
        require_text(&segment.segment_id, "Git segment id")?;
        if segment.sha256.len() != 64
            || !segment.sha256.bytes().all(|byte| byte.is_ascii_hexdigit())
        {
            return Err(PostgresError::internal_message(
                "Git segment SHA-256 must contain 64 hexadecimal characters",
            ));
        }
        let result = self
            .db
            .execute(Statement::from_sql_and_values(
                DatabaseBackend::Postgres,
                "UPDATE scope_git_segment_uploads
                 SET state = 'ready', sha256 = $2, plaintext_bytes = $3,
                     encrypted_bytes = $4,
                     updated_at_unix = GREATEST(updated_at_unix, $5)
                 WHERE segment_id = $1 AND state = 'uploading' AND encoding_version = $6",
                [
                    segment.segment_id.clone().into(),
                    segment.sha256.clone().into(),
                    size(segment.plaintext_bytes, "Git segment plaintext size")?.into(),
                    size(encrypted_bytes, "Git segment encrypted size")?.into(),
                    timestamp(now_unix, "Git segment ready time")?.into(),
                    i32::try_from(segment.encoding_version)
                        .map_err(|_| {
                            PostgresError::internal_message(
                                "Git segment encoding version exceeds database integer",
                            )
                        })?
                        .into(),
                ],
            ))
            .await
            .map_err(PostgresError::internal)?;
        require_one_transition(result.rows_affected(), &segment.segment_id, "ready")
    }

    pub async fn mark_git_segment_upload_published(
        &self,
        segment_id: &str,
        now_unix: u64,
    ) -> Result<(), PostgresError> {
        let result = self
            .db
            .execute(Statement::from_sql_and_values(
                DatabaseBackend::Postgres,
                "UPDATE scope_git_segment_uploads uploads
                 SET state = 'published',
                     updated_at_unix = GREATEST(updated_at_unix, $2)
                 WHERE uploads.segment_id = $1 AND uploads.state = 'ready'
                   AND EXISTS (
                       SELECT 1 FROM scope_git_segments spans
                       WHERE spans.segment_id = uploads.segment_id
                   )",
                [
                    segment_id.into(),
                    timestamp(now_unix, "Git segment publication time")?.into(),
                ],
            ))
            .await
            .map_err(PostgresError::internal)?;
        require_one_transition(result.rows_affected(), segment_id, "published")
    }

    pub async fn mark_git_segment_upload_deleting(
        &self,
        segment_id: &str,
        now_unix: u64,
    ) -> Result<(), PostgresError> {
        transition(
            self.db.as_ref(),
            segment_id,
            "state IN ('uploading', 'ready', 'published') AND NOT EXISTS (
                SELECT 1 FROM scope_git_segments spans
                WHERE spans.segment_id = scope_git_segment_uploads.segment_id
            ) AND NOT EXISTS (
                SELECT 1 FROM scope_git_segment_references refs
                WHERE refs.segment_id = scope_git_segment_uploads.segment_id
            )",
            "deleting",
            now_unix,
        )
        .await
    }

    pub async fn abandon_git_segment_upload(
        &self,
        segment_id: &str,
        now_unix: u64,
    ) -> Result<bool, PostgresError> {
        require_text(segment_id, "Git segment id")?;
        let result = self
            .db
            .execute(Statement::from_sql_and_values(
                DatabaseBackend::Postgres,
                "UPDATE scope_git_segment_uploads uploads
                 SET state = 'deleting',
                     updated_at_unix = GREATEST(updated_at_unix, $2)
                 WHERE uploads.segment_id = $1
                   AND uploads.state IN ('uploading', 'ready')
                   AND NOT EXISTS (
                       SELECT 1 FROM scope_git_segments spans
                       WHERE spans.segment_id = uploads.segment_id
                   )",
                [
                    segment_id.into(),
                    timestamp(now_unix, "Git segment abandonment time")?.into(),
                ],
            ))
            .await
            .map_err(PostgresError::internal)?;
        Ok(result.rows_affected() == 1)
    }

    pub async fn mark_git_segment_upload_deleted(
        &self,
        segment_id: &str,
        now_unix: u64,
    ) -> Result<(), PostgresError> {
        transition(
            self.db.as_ref(),
            segment_id,
            "state = 'deleting'",
            "deleted",
            now_unix,
        )
        .await
    }

    pub async fn load_stale_git_segment_uploads(
        &self,
        updated_before_unix: u64,
        limit: u64,
    ) -> Result<Vec<GitSegmentUpload>, PostgresError> {
        if limit == 0 {
            return Ok(Vec::new());
        }
        entities::git_segment_upload::Entity::find()
            .filter(entities::git_segment_upload::Column::State.is_in([
                "uploading",
                "ready",
                "deleting",
            ]))
            .filter(
                entities::git_segment_upload::Column::UpdatedAtUnix.lte(timestamp(
                    updated_before_unix,
                    "Git segment recovery cutoff",
                )?),
            )
            .order_by_asc(entities::git_segment_upload::Column::UpdatedAtUnix)
            .order_by_asc(entities::git_segment_upload::Column::SegmentId)
            .limit(limit)
            .all(self.db.as_ref())
            .await
            .map_err(PostgresError::internal)?
            .into_iter()
            .map(entities::git_segment_upload::Model::try_into_domain)
            .collect()
    }
}

pub(super) async fn load_git_pack_spans<C>(
    conn: &C,
    repo_id: &str,
) -> Result<Vec<GitPackSpan>, PostgresError>
where
    C: ConnectionTrait,
{
    let rows = entities::git_pack_span::Entity::find()
        .filter(entities::git_pack_span::Column::RepoId.eq(repo_id))
        .order_by_asc(entities::git_pack_span::Column::FirstSequence)
        .all(conn)
        .await
        .map_err(PostgresError::internal)?;
    let segment_ids = rows
        .iter()
        .map(|row| row.segment_id.clone())
        .collect::<Vec<_>>();
    let uploads = if segment_ids.is_empty() {
        Vec::new()
    } else {
        entities::git_segment_upload::Entity::find()
            .filter(entities::git_segment_upload::Column::SegmentId.is_in(segment_ids))
            .all(conn)
            .await
            .map_err(PostgresError::internal)?
    };
    let mut segments = uploads
        .into_iter()
        .map(|upload| Ok((upload.segment_id.clone(), upload.ready_segment_ref()?)))
        .collect::<Result<BTreeMap<_, _>, PostgresError>>()?;
    rows.into_iter()
        .map(|row| {
            let segment = segments.remove(&row.segment_id).ok_or_else(|| {
                PostgresError::internal_message(format!(
                    "Git pack span references missing segment {}",
                    row.segment_id
                ))
            })?;
            row.try_into_domain(segment)
        })
        .collect()
}

pub(super) async fn publish_git_segment<C>(
    conn: &C,
    repo_id: &str,
    segment: &GitSegmentRef,
    now_unix: u64,
) -> Result<(), PostgresError>
where
    C: ConnectionTrait,
{
    let result = conn
        .execute(Statement::from_sql_and_values(
            DatabaseBackend::Postgres,
            "UPDATE scope_git_segment_uploads
             SET state = 'published',
                 updated_at_unix = GREATEST(updated_at_unix, $6)
             WHERE segment_id = $1 AND repo_id = $2 AND state IN ('ready', 'published')
               AND sha256 = $3 AND plaintext_bytes = $4 AND encoding_version = $5",
            [
                segment.segment_id.clone().into(),
                repo_id.into(),
                segment.sha256.clone().into(),
                size(segment.plaintext_bytes, "Git segment plaintext size")?.into(),
                i32::try_from(segment.encoding_version)
                    .map_err(|_| {
                        PostgresError::internal_message(
                            "Git segment encoding version exceeds database integer",
                        )
                    })?
                    .into(),
                timestamp(now_unix, "Git segment publication time")?.into(),
            ],
        ))
        .await
        .map_err(PostgresError::internal)?;
    require_one_transition(result.rows_affected(), &segment.segment_id, "published")
}

pub(super) async fn retire_git_segment<C>(
    conn: &C,
    segment_id: &str,
    now_unix: u64,
) -> Result<bool, PostgresError>
where
    C: ConnectionTrait,
{
    let result = conn
        .execute(Statement::from_sql_and_values(
            DatabaseBackend::Postgres,
            "UPDATE scope_git_segment_uploads uploads
             SET state = 'deleting',
                 updated_at_unix = GREATEST(updated_at_unix, $2)
             WHERE uploads.segment_id = $1
               AND uploads.state IN ('ready', 'published')
               AND NOT EXISTS (
                   SELECT 1 FROM scope_git_segments spans
                   WHERE spans.segment_id = uploads.segment_id
               )
               AND NOT EXISTS (
                   SELECT 1 FROM scope_git_segment_references refs
                   WHERE refs.segment_id = uploads.segment_id
               )",
            [
                segment_id.into(),
                timestamp(now_unix, "Git segment retirement time")?.into(),
            ],
        ))
        .await
        .map_err(PostgresError::internal)?;
    Ok(result.rows_affected() == 1)
}

pub(super) async fn insert_git_segment_references<C>(
    conn: &C,
    ref_kind: &str,
    ref_id: &str,
    segments: impl IntoIterator<Item = &GitSegmentRef>,
) -> Result<(), PostgresError>
where
    C: ConnectionTrait,
{
    require_reference_kind(ref_kind)?;
    require_text(ref_id, "Git segment reference id")?;
    for segment in segments {
        conn.execute(Statement::from_sql_and_values(
            DatabaseBackend::Postgres,
            "INSERT INTO scope_git_segment_references (segment_id, ref_kind, ref_id)
             VALUES ($1, $2, $3) ON CONFLICT DO NOTHING",
            [
                segment.segment_id.clone().into(),
                ref_kind.into(),
                ref_id.into(),
            ],
        ))
        .await
        .map_err(PostgresError::internal)?;
    }
    Ok(())
}

pub(super) async fn release_git_segment_references<C>(
    conn: &C,
    ref_kind: &str,
    ref_id: &str,
    now_unix: u64,
) -> Result<(), PostgresError>
where
    C: ConnectionTrait,
{
    require_reference_kind(ref_kind)?;
    require_text(ref_id, "Git segment reference id")?;
    conn.execute(Statement::from_sql_and_values(
        DatabaseBackend::Postgres,
        "WITH released AS (
             DELETE FROM scope_git_segment_references
             WHERE ref_kind = $1 AND ref_id = $2
             RETURNING segment_id
         )
         UPDATE scope_git_segment_uploads uploads
         SET state = 'deleting',
             updated_at_unix = GREATEST(updated_at_unix, $3)
         WHERE uploads.segment_id IN (SELECT segment_id FROM released)
           AND uploads.state = 'published'
           AND NOT EXISTS (
               SELECT 1 FROM scope_git_segments spans
               WHERE spans.segment_id = uploads.segment_id
           )
           AND NOT EXISTS (
               SELECT 1 FROM scope_git_segment_references refs
               WHERE refs.segment_id = uploads.segment_id
                 AND NOT (refs.ref_kind = $1 AND refs.ref_id = $2)
           )",
        [
            ref_kind.into(),
            ref_id.into(),
            timestamp(now_unix, "Git segment reference release time")?.into(),
        ],
    ))
    .await
    .map_err(PostgresError::internal)?;
    Ok(())
}

async fn transition<C>(
    conn: &C,
    segment_id: &str,
    from_predicate: &str,
    to: &str,
    now_unix: u64,
) -> Result<(), PostgresError>
where
    C: ConnectionTrait,
{
    require_text(segment_id, "Git segment id")?;
    let statement = format!(
        "UPDATE scope_git_segment_uploads
         SET state = $2, updated_at_unix = GREATEST(updated_at_unix, $3)
         WHERE segment_id = $1 AND {from_predicate}"
    );
    let result = conn
        .execute(Statement::from_sql_and_values(
            DatabaseBackend::Postgres,
            statement,
            [
                segment_id.into(),
                to.into(),
                timestamp(now_unix, "Git segment transition time")?.into(),
            ],
        ))
        .await
        .map_err(PostgresError::internal)?;
    require_one_transition(result.rows_affected(), segment_id, to)
}

fn require_one_transition(
    rows_affected: u64,
    segment_id: &str,
    target: &str,
) -> Result<(), PostgresError> {
    if rows_affected == 1 {
        Ok(())
    } else {
        Err(PostgresError::conflict(format!(
            "Git segment {segment_id} cannot transition to {target}"
        )))
    }
}

fn require_text(value: &str, field: &str) -> Result<(), PostgresError> {
    if value.trim().is_empty() {
        Err(PostgresError::internal_message(format!(
            "{field} is required"
        )))
    } else {
        Ok(())
    }
}

fn require_reference_kind(ref_kind: &str) -> Result<(), PostgresError> {
    if matches!(ref_kind, "push_trigger_source" | "run_source") {
        Ok(())
    } else {
        Err(PostgresError::internal_message(format!(
            "unsupported Git segment reference kind {ref_kind}"
        )))
    }
}

fn timestamp(value: u64, field: &str) -> Result<i64, PostgresError> {
    i64::try_from(value)
        .map_err(|_| PostgresError::internal_message(format!("{field} exceeds database bigint")))
}

fn size(value: u64, field: &str) -> Result<i64, PostgresError> {
    i64::try_from(value)
        .map_err(|_| PostgresError::internal_message(format!("{field} exceeds database bigint")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::{MetadataStore, TestDatabaseTarget};
    use scope_domain::repository::git::{GitSegmentRef, GitSegmentUploadState};
    use sea_orm::{ActiveModelTrait, ConnectionTrait, IntoActiveModel};
    use std::time::Duration;

    async fn store_with_repository(repo_id: &str) -> MetadataStore {
        let store =
            MetadataStore::connect_fresh_for_tests(&TestDatabaseTarget::required().unwrap())
                .unwrap();
        store
            .db
            .execute_unprepared(&format!(
                "INSERT INTO scope_users (id, handle, email, email_verified)
                 VALUES ('segment_user', 'segment-user', 'segment@scope.test', TRUE);
                 INSERT INTO scope_repositories (
                    id, owner_handle, name, owner_user_id, publication_state,
                    change_version, repo_config, policy
                 ) VALUES (
                    '{repo_id}', 'segment-user', 'repo', 'segment_user', 'Ready', 1,
                    '{{\"kind\":\"scope.repo-config\",\"version\":1,\"visibility\":{{\"default\":\"private\",\"rules\":[]}}}}'::jsonb,
                    '{{\"default_visibility\":\"Private\",\"rules\":[]}}'::jsonb
                 )"
            ))
            .await
            .unwrap();
        store
    }

    fn segment() -> GitSegmentRef {
        segment_named("segment-1", 'a')
    }

    fn segment_named(segment_id: &str, digest: char) -> GitSegmentRef {
        GitSegmentRef {
            segment_id: segment_id.to_string(),
            sha256: digest.to_string().repeat(64),
            plaintext_bytes: 1_024,
            encoding_version: 2,
        }
    }

    #[tokio::test]
    async fn upload_ledger_enforces_publication_and_recovery_states() {
        let store = store_with_repository("segment-user/repo").await;
        let repositories = store.repositories();
        let segment = segment();
        repositories
            .begin_git_segment_upload(
                "segment-user/repo",
                &segment.segment_id,
                "git/segments/v2/segment-user/repo/segment-1",
                segment.encoding_version,
                10,
            )
            .await
            .unwrap();
        repositories
            .mark_git_segment_upload_ready(&segment, 1_100, 11)
            .await
            .unwrap();

        let stale = repositories
            .load_stale_git_segment_uploads(11, 10)
            .await
            .unwrap();
        assert_eq!(stale.len(), 1);
        assert_eq!(stale[0].state, GitSegmentUploadState::Ready);

        let span = GitPackSpan {
            first_sequence: 1,
            last_sequence: 1,
            geometric_tier: 0,
            base_oid: None,
            head_oid: "a".repeat(40),
            segment: segment.clone(),
        };
        entities::git_pack_span::Model::from_domain("segment-user/repo", &span)
            .unwrap()
            .into_active_model()
            .insert(store.db.as_ref())
            .await
            .unwrap();
        repositories
            .mark_git_segment_upload_published(&segment.segment_id, 12)
            .await
            .unwrap();
        assert!(
            !repositories
                .abandon_git_segment_upload(&segment.segment_id, 13)
                .await
                .unwrap()
        );
        assert!(
            repositories
                .mark_git_segment_upload_deleting(&segment.segment_id, 13)
                .await
                .is_err()
        );

        entities::git_pack_span::Entity::delete_many()
            .exec(store.db.as_ref())
            .await
            .unwrap();
        repositories
            .mark_git_segment_upload_deleting(&segment.segment_id, 13)
            .await
            .unwrap();
        repositories
            .mark_git_segment_upload_deleted(&segment.segment_id, 14)
            .await
            .unwrap();
        assert!(
            repositories
                .load_stale_git_segment_uploads(14, 10)
                .await
                .unwrap()
                .is_empty()
        );
    }

    #[tokio::test]
    async fn repository_git_write_lease_is_session_scoped() {
        let store = store_with_repository("segment-user/repo").await;
        let repositories = store.repositories();
        let first = repositories
            .acquire_git_write_lease("segment-user/repo")
            .await
            .unwrap();
        let waiting_store = repositories.clone();
        let mut waiting = tokio::spawn(async move {
            waiting_store
                .acquire_git_write_lease("segment-user/repo")
                .await
                .unwrap()
        });
        assert!(
            tokio::time::timeout(Duration::from_millis(100), &mut waiting)
                .await
                .is_err()
        );
        first.release().await;
        let second = tokio::time::timeout(Duration::from_secs(2), waiting)
            .await
            .unwrap()
            .unwrap();
        second.release().await;
    }

    #[tokio::test]
    async fn repository_deletion_keeps_segment_ledger_for_physical_cleanup() {
        let store = store_with_repository("segment-user/repo").await;
        let repositories = store.repositories();
        let segment = segment();
        repositories
            .begin_git_segment_upload(
                "segment-user/repo",
                &segment.segment_id,
                "git/segments/v2/segment-user/repo/segment-1",
                segment.encoding_version,
                10,
            )
            .await
            .unwrap();
        repositories
            .mark_git_segment_upload_ready(&segment, 1_100, 11)
            .await
            .unwrap();
        entities::git_pack_span::Model::from_domain(
            "segment-user/repo",
            &GitPackSpan {
                first_sequence: 1,
                last_sequence: 1,
                geometric_tier: 0,
                base_oid: None,
                head_oid: "a".repeat(40),
                segment: segment.clone(),
            },
        )
        .unwrap()
        .into_active_model()
        .insert(store.db.as_ref())
        .await
        .unwrap();
        repositories
            .mark_git_segment_upload_published(&segment.segment_id, 12)
            .await
            .unwrap();

        repositories
            .delete_repo(
                "segment-user",
                "repo",
                "segment_user",
                20,
                &crate::db::generated_ids::test_generated_id,
            )
            .await
            .unwrap();

        let recoverable = repositories
            .load_stale_git_segment_uploads(20, 10)
            .await
            .unwrap();
        assert_eq!(recoverable.len(), 1);
        assert_eq!(recoverable[0].state, GitSegmentUploadState::Deleting);
    }

    #[tokio::test]
    async fn trigger_and_run_pins_block_compaction_retirement() {
        let store = store_with_repository("segment-user/repo").await;
        let repositories = store.repositories();
        for (index, ref_kind) in ["push_trigger_source", "run_source"]
            .into_iter()
            .enumerate()
        {
            let segment = segment_named(&format!("segment-pin-{index}"), ['b', 'c'][index]);
            repositories
                .begin_git_segment_upload(
                    "segment-user/repo",
                    &segment.segment_id,
                    &format!("git/segments/v2/segment-user/repo/{}", segment.segment_id),
                    segment.encoding_version,
                    10,
                )
                .await
                .unwrap();
            repositories
                .mark_git_segment_upload_ready(&segment, 1_100, 11)
                .await
                .unwrap();
            entities::git_pack_span::Model::from_domain(
                "segment-user/repo",
                &GitPackSpan {
                    first_sequence: index as u64 + 1,
                    last_sequence: index as u64 + 1,
                    geometric_tier: 0,
                    base_oid: (index > 0).then(|| "a".repeat(40)),
                    head_oid: ['a', 'd'][index].to_string().repeat(40),
                    segment: segment.clone(),
                },
            )
            .unwrap()
            .into_active_model()
            .insert(store.db.as_ref())
            .await
            .unwrap();
            repositories
                .mark_git_segment_upload_published(&segment.segment_id, 12)
                .await
                .unwrap();
            insert_git_segment_references(
                store.db.as_ref(),
                ref_kind,
                &format!("pin-{index}"),
                [&segment],
            )
            .await
            .unwrap();

            entities::git_pack_span::Entity::delete_by_id((
                "segment-user/repo".to_string(),
                i64::try_from(index + 1).unwrap(),
            ))
            .exec(store.db.as_ref())
            .await
            .unwrap();
            assert!(
                !retire_git_segment(store.db.as_ref(), &segment.segment_id, 13)
                    .await
                    .unwrap()
            );
            let row = entities::git_segment_upload::Entity::find_by_id(&segment.segment_id)
                .one(store.db.as_ref())
                .await
                .unwrap()
                .unwrap();
            assert_eq!(
                row.try_into_domain().unwrap().state,
                GitSegmentUploadState::Published
            );

            release_git_segment_references(
                store.db.as_ref(),
                ref_kind,
                &format!("pin-{index}"),
                14,
            )
            .await
            .unwrap();
            let row = entities::git_segment_upload::Entity::find_by_id(&segment.segment_id)
                .one(store.db.as_ref())
                .await
                .unwrap()
                .unwrap();
            assert_eq!(
                row.try_into_domain().unwrap().state,
                GitSegmentUploadState::Deleting
            );
        }
    }
}
