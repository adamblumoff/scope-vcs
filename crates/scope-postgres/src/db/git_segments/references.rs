use super::ledger::{require_text, timestamp};
use crate::error::PostgresError;
use scope_domain::repository::git::GitSegmentRef;
use sea_orm::{ConnectionTrait, DatabaseBackend, Statement};

pub(in crate::db) async fn insert_git_segment_references<'a, C>(
    conn: &C,
    ref_kind: &str,
    ref_id: &str,
    segments: impl IntoIterator<Item = &'a GitSegmentRef>,
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

pub(in crate::db) async fn release_git_segment_references<C>(
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
           AND uploads.state = 'retained'
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

fn require_reference_kind(ref_kind: &str) -> Result<(), PostgresError> {
    if matches!(ref_kind, "push_trigger_source" | "run_source") {
        Ok(())
    } else {
        Err(PostgresError::internal_message(format!(
            "unsupported Git segment reference kind {ref_kind}"
        )))
    }
}
