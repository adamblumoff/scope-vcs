use super::super::{RepositoryStore, entities};
use crate::error::PostgresError;
use scope_domain::repository::git::{GitSegmentRef, GitSegmentUpload};
use sea_orm::{
    ColumnTrait, ConnectionTrait, DatabaseBackend, EntityTrait, QueryFilter, QueryOrder,
    QuerySelect, Statement,
};

impl RepositoryStore {
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

    pub async fn touch_git_segment_upload(
        &self,
        segment_id: &str,
        now_unix: u64,
    ) -> Result<bool, PostgresError> {
        require_text(segment_id, "Git segment id")?;
        let result = self
            .db
            .execute(Statement::from_sql_and_values(
                DatabaseBackend::Postgres,
                "UPDATE scope_git_segment_uploads
                 SET updated_at_unix = GREATEST(updated_at_unix, $2)
                 WHERE segment_id = $1 AND state IN ('uploading', 'ready')",
                [
                    segment_id.into(),
                    timestamp(now_unix, "Git segment upload heartbeat time")?.into(),
                ],
            ))
            .await
            .map_err(PostgresError::internal)?;
        Ok(result.rows_affected() == 1)
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

pub(super) fn require_one_transition(
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

pub(super) fn require_text(value: &str, field: &str) -> Result<(), PostgresError> {
    if value.trim().is_empty() {
        Err(PostgresError::internal_message(format!(
            "{field} is required"
        )))
    } else {
        Ok(())
    }
}

pub(super) fn timestamp(value: u64, field: &str) -> Result<i64, PostgresError> {
    i64::try_from(value)
        .map_err(|_| PostgresError::internal_message(format!("{field} exceeds database bigint")))
}

pub(super) fn size(value: u64, field: &str) -> Result<i64, PostgresError> {
    i64::try_from(value)
        .map_err(|_| PostgresError::internal_message(format!("{field} exceeds database bigint")))
}
