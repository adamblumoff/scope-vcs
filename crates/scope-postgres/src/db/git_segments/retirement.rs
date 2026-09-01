use super::ledger::timestamp;
use crate::error::PostgresError;
use sea_orm::{ConnectionTrait, DatabaseBackend, Statement};

pub(in crate::db) async fn retire_git_segment<C>(
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
             SET state = CASE
                     WHEN EXISTS (
                         SELECT 1 FROM scope_git_segment_references refs
                         WHERE refs.segment_id = uploads.segment_id
                     ) THEN 'retained'
                     ELSE 'deleting'
                 END,
                 updated_at_unix = GREATEST(updated_at_unix, $2)
             WHERE uploads.segment_id = $1
               AND uploads.state IN ('ready', 'published', 'retained')
               AND NOT EXISTS (
                   SELECT 1 FROM scope_git_segments spans
                   WHERE spans.segment_id = uploads.segment_id
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
