use super::{
    MetadataStore, MetadataStoreInner, acquire_metadata_write_lock, entities,
    projection_read_models::save_live_projection_read_models, repository_from_model, run_api_db_on,
};
use crate::{error::ApiError, persistence::unix_now};
#[cfg(any(test, feature = "test-support"))]
use sea_orm::PaginatorTrait;
use sea_orm::{
    ColumnTrait, Condition, ConnectionTrait, EntityTrait, IntoActiveModel, QueryFilter, QueryOrder,
    TransactionTrait, TryInsertResult,
    sea_query::{Expr, OnConflict},
};
use std::sync::Arc;

const PROJECTION_READ_MODEL_REBUILD: &str = "projection_read_model_rebuild";
const JOB_READY: &str = "ready";
const JOB_RUNNING: &str = "running";
const JOB_SUCCEEDED: &str = "succeeded";
const JOB_FAILED: &str = "failed";
const DEFAULT_JOB_LEASE_SECS: i64 = 60;
const MAX_RETRY_DELAY_SECS: i64 = 300;
const MAX_JOB_ATTEMPTS: i64 = 12;
const SUCCEEDED_JOB_RETENTION_SECS: i64 = 7 * 24 * 60 * 60;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct OutboxRunSummary {
    pub claimed: usize,
    pub completed: usize,
    pub failed: usize,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct OutboxJobCounts {
    pub ready: usize,
    pub running: usize,
    pub succeeded: usize,
    pub failed: usize,
    pub total: usize,
}

#[derive(Clone, Debug)]
struct ClaimedOutboxJob {
    id: String,
    kind: String,
    repo_id: String,
    repo_version: i64,
    attempts: i64,
}

impl MetadataStore {
    pub fn run_ready_outbox_jobs(
        &self,
        worker_id: &str,
        limit: usize,
    ) -> Result<OutboxRunSummary, ApiError> {
        if limit == 0 {
            return Ok(OutboxRunSummary::default());
        }

        match self.inner.as_ref() {
            MetadataStoreInner::Postgres { db, runtime } => {
                let db = Arc::clone(db);
                let worker_id = worker_id.to_string();
                run_api_db_on(runtime, async move {
                    let mut summary = OutboxRunSummary::default();
                    for _ in 0..limit {
                        let Some(job) =
                            claim_next_ready_job(db.as_ref(), &worker_id, DEFAULT_JOB_LEASE_SECS)
                                .await?
                        else {
                            break;
                        };
                        summary.claimed += 1;

                        match execute_outbox_job(db.as_ref(), &job).await {
                            Ok(()) => {
                                complete_outbox_job(db.as_ref(), &job, &worker_id).await?;
                                summary.completed += 1;
                            }
                            Err(error) => {
                                let message = error.message;
                                let attempts = next_retry_attempt(job.attempts);
                                if is_terminal_retry_attempt(attempts) {
                                    tracing::error!(
                                        job_id = %job.id,
                                        kind = %job.kind,
                                        attempts,
                                        max_attempts = MAX_JOB_ATTEMPTS,
                                        error = %message,
                                        "outbox job exhausted retries; marking failed"
                                    );
                                } else {
                                    tracing::warn!(
                                        job_id = %job.id,
                                        kind = %job.kind,
                                        attempts,
                                        max_attempts = MAX_JOB_ATTEMPTS,
                                        next_retry_delay_secs = retry_delay_seconds(attempts),
                                        error = %message,
                                        "outbox job failed; scheduling retry"
                                    );
                                }
                                fail_outbox_job(db.as_ref(), &job, &worker_id, message).await?;
                                summary.failed += 1;
                            }
                        }
                    }
                    Ok(summary)
                })
            }
            #[cfg(any(test, feature = "memory-metadata"))]
            MetadataStoreInner::Memory(_) => Ok(OutboxRunSummary::default()),
        }
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn outbox_job_counts_for_tests(&self) -> Result<OutboxJobCounts, ApiError> {
        match self.inner.as_ref() {
            MetadataStoreInner::Postgres { db, runtime } => {
                let db = Arc::clone(db);
                run_api_db_on(runtime, async move {
                    let rows = entities::outbox_job::Entity::find()
                        .all(db.as_ref())
                        .await
                        .map_err(ApiError::internal)?;
                    Ok(outbox_job_counts(rows))
                })
            }
            MetadataStoreInner::Memory(_) => Ok(OutboxJobCounts::default()),
        }
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn projection_read_model_count_for_tests(&self, repo_id: &str) -> Result<usize, ApiError> {
        let repo_id = repo_id.to_string();
        match self.inner.as_ref() {
            MetadataStoreInner::Postgres { db, runtime } => {
                let db = Arc::clone(db);
                run_api_db_on(runtime, async move {
                    entities::projection_read_model::Entity::find()
                        .filter(entities::projection_read_model::Column::RepoId.eq(repo_id))
                        .count(db.as_ref())
                        .await
                        .map(|count| count as usize)
                        .map_err(ApiError::internal)
                })
            }
            MetadataStoreInner::Memory(_) => Ok(0),
        }
    }
}

pub async fn enqueue_projection_read_model_rebuild<C>(
    conn: &C,
    repo: &crate::domain::store::StoredRepository,
) -> Result<(), ApiError>
where
    C: ConnectionTrait,
{
    let now = unix_now()?;
    let job = entities::outbox_job::Model::projection_read_model_rebuild(
        new_outbox_job_id()?,
        repo,
        now,
    )?;
    match entities::outbox_job::Entity::insert(job.into_active_model())
        .on_conflict(
            OnConflict::column(entities::outbox_job::Column::IdempotencyKey)
                .do_nothing()
                .to_owned(),
        )
        .do_nothing()
        .exec(conn)
        .await
        .map_err(ApiError::internal)?
    {
        TryInsertResult::Empty | TryInsertResult::Conflicted | TryInsertResult::Inserted(_) => {}
    }
    Ok(())
}

async fn claim_next_ready_job<C>(
    conn: &C,
    worker_id: &str,
    lease_seconds: i64,
) -> Result<Option<ClaimedOutboxJob>, ApiError>
where
    C: ConnectionTrait + TransactionTrait,
{
    let tx = conn.begin().await.map_err(ApiError::internal)?;
    acquire_metadata_write_lock(&tx).await?;
    let now = now_i64()?;
    let runnable = Condition::any()
        .add(
            Condition::all()
                .add(entities::outbox_job::Column::State.eq(JOB_READY))
                .add(entities::outbox_job::Column::NextRunAtUnix.lte(now)),
        )
        .add(
            Condition::all()
                .add(entities::outbox_job::Column::State.eq(JOB_RUNNING))
                .add(entities::outbox_job::Column::LeaseExpiresAtUnix.lte(now)),
        );

    let Some(job) = entities::outbox_job::Entity::find()
        .filter(entities::outbox_job::Column::CompletedAtUnix.is_null())
        .filter(runnable)
        .order_by_asc(entities::outbox_job::Column::NextRunAtUnix)
        .order_by_asc(entities::outbox_job::Column::CreatedAtUnix)
        .order_by_asc(entities::outbox_job::Column::Id)
        .one(&tx)
        .await
        .map_err(ApiError::internal)?
    else {
        tx.commit().await.map_err(ApiError::internal)?;
        return Ok(None);
    };

    entities::outbox_job::Entity::update_many()
        .filter(entities::outbox_job::Column::Id.eq(job.id.clone()))
        .filter(entities::outbox_job::Column::CompletedAtUnix.is_null())
        .col_expr(
            entities::outbox_job::Column::State,
            Expr::value(JOB_RUNNING),
        )
        .col_expr(
            entities::outbox_job::Column::LeaseOwner,
            Expr::value(Some(worker_id.to_string())),
        )
        .col_expr(
            entities::outbox_job::Column::LeaseExpiresAtUnix,
            Expr::value(Some(now.saturating_add(lease_seconds))),
        )
        .col_expr(
            entities::outbox_job::Column::UpdatedAtUnix,
            Expr::value(now),
        )
        .exec(&tx)
        .await
        .map_err(ApiError::internal)?;
    tx.commit().await.map_err(ApiError::internal)?;

    Ok(Some(ClaimedOutboxJob {
        id: job.id,
        kind: job.kind,
        repo_id: job.repo_id,
        repo_version: job.repo_version,
        attempts: job.attempts,
    }))
}

async fn execute_outbox_job<C>(conn: &C, job: &ClaimedOutboxJob) -> Result<(), ApiError>
where
    C: ConnectionTrait + TransactionTrait,
{
    match job.kind.as_str() {
        PROJECTION_READ_MODEL_REBUILD => {
            rebuild_live_projection_read_models_for_job(conn, job).await
        }
        kind => Err(ApiError::internal_message(format!(
            "unknown outbox job kind {kind}"
        ))),
    }
}

async fn rebuild_live_projection_read_models_for_job<C>(
    conn: &C,
    job: &ClaimedOutboxJob,
) -> Result<(), ApiError>
where
    C: ConnectionTrait + TransactionTrait,
{
    let tx = conn.begin().await.map_err(ApiError::internal)?;
    acquire_metadata_write_lock(&tx).await?;
    let Some(repo) = entities::repository::Entity::find_by_id(job.repo_id.clone())
        .one(&tx)
        .await
        .map_err(ApiError::internal)?
    else {
        tx.commit().await.map_err(ApiError::internal)?;
        return Ok(());
    };
    if repo.change_version != job.repo_version {
        tx.commit().await.map_err(ApiError::internal)?;
        return Ok(());
    }
    let repo = repository_from_model(&tx, repo).await?;
    save_live_projection_read_models(&tx, &repo).await?;
    tx.commit().await.map_err(ApiError::internal)?;
    Ok(())
}

async fn complete_outbox_job<C>(
    conn: &C,
    job: &ClaimedOutboxJob,
    worker_id: &str,
) -> Result<(), ApiError>
where
    C: ConnectionTrait,
{
    let now = now_i64()?;
    entities::outbox_job::Entity::update_many()
        .filter(entities::outbox_job::Column::Id.eq(job.id.clone()))
        .filter(entities::outbox_job::Column::LeaseOwner.eq(worker_id.to_string()))
        .col_expr(
            entities::outbox_job::Column::State,
            Expr::value(JOB_SUCCEEDED),
        )
        .col_expr(
            entities::outbox_job::Column::LeaseOwner,
            Expr::value(Option::<String>::None),
        )
        .col_expr(
            entities::outbox_job::Column::LeaseExpiresAtUnix,
            Expr::value(Option::<i64>::None),
        )
        .col_expr(
            entities::outbox_job::Column::LastError,
            Expr::value(Option::<String>::None),
        )
        .col_expr(
            entities::outbox_job::Column::CompletedAtUnix,
            Expr::value(Some(now)),
        )
        .col_expr(
            entities::outbox_job::Column::UpdatedAtUnix,
            Expr::value(now),
        )
        .exec(conn)
        .await
        .map_err(ApiError::internal)?;
    prune_succeeded_outbox_jobs(conn, now).await?;
    Ok(())
}

async fn fail_outbox_job<C>(
    conn: &C,
    job: &ClaimedOutboxJob,
    worker_id: &str,
    error: String,
) -> Result<(), ApiError>
where
    C: ConnectionTrait,
{
    let now = now_i64()?;
    let attempts = next_retry_attempt(job.attempts);
    let terminal = is_terminal_retry_attempt(attempts);
    let next_run_at = if terminal {
        now
    } else {
        now.saturating_add(retry_delay_seconds(attempts))
    };
    let completed_at_unix = terminal.then_some(now);
    let state = if terminal { JOB_FAILED } else { JOB_READY };
    entities::outbox_job::Entity::update_many()
        .filter(entities::outbox_job::Column::Id.eq(job.id.clone()))
        .filter(entities::outbox_job::Column::LeaseOwner.eq(worker_id.to_string()))
        .filter(entities::outbox_job::Column::CompletedAtUnix.is_null())
        .col_expr(entities::outbox_job::Column::State, Expr::value(state))
        .col_expr(
            entities::outbox_job::Column::Attempts,
            Expr::value(attempts),
        )
        .col_expr(
            entities::outbox_job::Column::NextRunAtUnix,
            Expr::value(next_run_at),
        )
        .col_expr(
            entities::outbox_job::Column::LeaseOwner,
            Expr::value(Option::<String>::None),
        )
        .col_expr(
            entities::outbox_job::Column::LeaseExpiresAtUnix,
            Expr::value(Option::<i64>::None),
        )
        .col_expr(
            entities::outbox_job::Column::LastError,
            Expr::value(Some(truncate_error(error))),
        )
        .col_expr(
            entities::outbox_job::Column::UpdatedAtUnix,
            Expr::value(now),
        )
        .col_expr(
            entities::outbox_job::Column::CompletedAtUnix,
            Expr::value(completed_at_unix),
        )
        .exec(conn)
        .await
        .map_err(ApiError::internal)?;
    Ok(())
}

async fn prune_succeeded_outbox_jobs<C>(conn: &C, now: i64) -> Result<(), ApiError>
where
    C: ConnectionTrait,
{
    let cutoff = now.saturating_sub(SUCCEEDED_JOB_RETENTION_SECS);
    entities::outbox_job::Entity::delete_many()
        .filter(entities::outbox_job::Column::State.eq(JOB_SUCCEEDED))
        .filter(entities::outbox_job::Column::CompletedAtUnix.lte(cutoff))
        .exec(conn)
        .await
        .map_err(ApiError::internal)?;
    Ok(())
}

#[cfg(any(test, feature = "test-support"))]
fn outbox_job_counts(rows: Vec<entities::outbox_job::Model>) -> OutboxJobCounts {
    let mut counts = OutboxJobCounts {
        total: rows.len(),
        ..OutboxJobCounts::default()
    };
    for row in rows {
        match row.state.as_str() {
            JOB_READY => counts.ready += 1,
            JOB_RUNNING => counts.running += 1,
            JOB_SUCCEEDED => counts.succeeded += 1,
            JOB_FAILED => counts.failed += 1,
            _ => {}
        }
    }
    counts
}

fn retry_delay_seconds(attempts: i64) -> i64 {
    let exponent = attempts.saturating_sub(1).clamp(0, 6) as u32;
    (5_i64.saturating_mul(2_i64.saturating_pow(exponent))).min(MAX_RETRY_DELAY_SECS)
}

fn next_retry_attempt(previous_attempts: i64) -> i64 {
    previous_attempts.saturating_add(1).max(1)
}

fn is_terminal_retry_attempt(attempts: i64) -> bool {
    attempts >= MAX_JOB_ATTEMPTS
}

fn truncate_error(error: String) -> String {
    const MAX_ERROR_CHARS: usize = 2_000;
    if error.chars().count() <= MAX_ERROR_CHARS {
        return error;
    }
    error.chars().take(MAX_ERROR_CHARS).collect()
}

fn now_i64() -> Result<i64, ApiError> {
    let now = unix_now()?;
    if now > i64::MAX as u64 {
        return Err(ApiError::internal_message("timestamp exceeds i64 range"));
    }
    Ok(now as i64)
}

fn new_outbox_job_id() -> Result<String, ApiError> {
    let mut bytes = [0_u8; 16];
    getrandom::fill(&mut bytes).map_err(|error| {
        ApiError::internal_message(format!("failed to generate outbox job id: {error}"))
    })?;
    Ok(format!("outbox_{}", hex::encode(bytes)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn retry_backoff_is_bounded() {
        assert_eq!(retry_delay_seconds(1), 5);
        assert_eq!(retry_delay_seconds(2), 10);
        assert_eq!(retry_delay_seconds(7), 300);
        assert_eq!(retry_delay_seconds(99), 300);
    }

    #[test]
    fn retry_policy_stops_at_max_attempts() {
        assert_eq!(next_retry_attempt(0), 1);
        assert!(!is_terminal_retry_attempt(MAX_JOB_ATTEMPTS - 1));
        assert!(is_terminal_retry_attempt(MAX_JOB_ATTEMPTS));
        assert!(is_terminal_retry_attempt(MAX_JOB_ATTEMPTS + 10));
    }
}
