use super::{
    MetadataStore, acquire_aggregate_lock, entities,
    projection_read_models::save_live_projection_read_models, repository_from_model,
};
use crate::{error::ApiError, persistence::unix_now};
#[cfg(any(test, feature = "test-support"))]
use sea_orm::PaginatorTrait;
use sea_orm::{
    ColumnTrait, Condition, ConnectionTrait, EntityTrait, IntoActiveModel, QueryFilter, QueryOrder,
    QuerySelect, TransactionTrait, TryInsertResult,
    sea_query::{Expr, LockBehavior, LockType, OnConflict},
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
    pub async fn run_ready_outbox_jobs(
        &self,
        worker_id: &str,
        limit: usize,
    ) -> Result<OutboxRunSummary, ApiError> {
        if limit == 0 {
            return Ok(OutboxRunSummary::default());
        }

        let db = Arc::clone(&self.db);
        let worker_id = worker_id.to_string();
        let mut summary = OutboxRunSummary::default();
        for _ in 0..limit {
            let Some(job) =
                claim_next_ready_job(db.as_ref(), &worker_id, DEFAULT_JOB_LEASE_SECS).await?
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
                    let attempts = next_retry_attempt(job.attempts)?;
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
    }

    #[cfg(any(test, feature = "test-support"))]
    pub async fn outbox_job_counts_for_tests(&self) -> Result<OutboxJobCounts, ApiError> {
        let db = Arc::clone(&self.db);
        let rows = entities::outbox_job::Entity::find()
            .all(db.as_ref())
            .await
            .map_err(ApiError::internal)?;
        Ok(outbox_job_counts(rows))
    }

    #[cfg(any(test, feature = "test-support"))]
    pub async fn projection_read_model_count_for_tests(
        &self,
        repo_id: &str,
    ) -> Result<usize, ApiError> {
        let repo_id = repo_id.to_string();
        let db = Arc::clone(&self.db);
        entities::projection_read_model::Entity::find()
            .filter(entities::projection_read_model::Column::RepoId.eq(repo_id))
            .count(db.as_ref())
            .await
            .map_err(ApiError::internal)
            .and_then(|count| {
                usize::try_from(count).map_err(|_| {
                    ApiError::internal_message("projection read-model count exceeds usize range")
                })
            })
    }
}

pub async fn enqueue_projection_read_model_rebuild<C>(
    conn: &C,
    repo_id: &str,
    repo_version: u64,
) -> Result<(), ApiError>
where
    C: ConnectionTrait,
{
    let now = unix_now()?;
    let job = entities::outbox_job::Model::projection_read_model_rebuild(
        new_outbox_job_id()?,
        repo_id,
        repo_version,
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
        .lock_with_behavior(LockType::Update, LockBehavior::SkipLocked)
        .one(&tx)
        .await
        .map_err(ApiError::internal)?
    else {
        tx.commit().await.map_err(ApiError::internal)?;
        return Ok(None);
    };

    let claimed = entities::outbox_job::Entity::update_many()
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
            Expr::value(Some(now.checked_add(lease_seconds).ok_or_else(|| {
                ApiError::internal_message("outbox lease expiry exceeds i64 range")
            })?)),
        )
        .col_expr(
            entities::outbox_job::Column::UpdatedAtUnix,
            Expr::value(now),
        )
        .exec(&tx)
        .await
        .map_err(ApiError::internal)?;
    if claimed.rows_affected != 1 {
        tx.rollback().await.map_err(ApiError::internal)?;
        return Ok(None);
    }
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
    acquire_aggregate_lock(&tx, "repository", &job.repo_id).await?;
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
    let completed = entities::outbox_job::Entity::update_many()
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
    if completed.rows_affected != 1 {
        let job_is_missing = entities::outbox_job::Entity::find_by_id(job.id.clone())
            .one(conn)
            .await
            .map_err(ApiError::internal)?
            .is_none();
        let repository_is_missing = entities::repository::Entity::find_by_id(job.repo_id.clone())
            .one(conn)
            .await
            .map_err(ApiError::internal)?
            .is_none();
        if job_is_missing && repository_is_missing {
            return Ok(());
        }
        return Err(ApiError::conflict(
            "outbox job lease was lost before completion",
        ));
    }
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
    let attempts = next_retry_attempt(job.attempts)?;
    let terminal = is_terminal_retry_attempt(attempts);
    let next_run_at = if terminal {
        now
    } else {
        now.checked_add(retry_delay_seconds(attempts))
            .ok_or_else(|| ApiError::internal_message("outbox retry time exceeds i64 range"))?
    };
    let completed_at_unix = terminal.then_some(now);
    let state = if terminal { JOB_FAILED } else { JOB_READY };
    let failed = entities::outbox_job::Entity::update_many()
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
    if failed.rows_affected != 1 {
        return Err(ApiError::conflict(
            "outbox job lease was lost before failure handling",
        ));
    }
    Ok(())
}

async fn prune_succeeded_outbox_jobs<C>(conn: &C, now: i64) -> Result<(), ApiError>
where
    C: ConnectionTrait,
{
    let cutoff = now.checked_sub(SUCCEEDED_JOB_RETENTION_SECS).unwrap_or(0);
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
    let exponent =
        u32::try_from(attempts.checked_sub(1).unwrap_or_default().clamp(0, 6)).unwrap_or_default();
    5_i64
        .checked_mul(2_i64.pow(exponent))
        .unwrap_or(MAX_RETRY_DELAY_SECS)
        .min(MAX_RETRY_DELAY_SECS)
}

fn next_retry_attempt(previous_attempts: i64) -> Result<i64, ApiError> {
    previous_attempts
        .checked_add(1)
        .filter(|attempts| *attempts >= 1)
        .ok_or_else(|| ApiError::internal_message("outbox attempt count is outside valid range"))
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
    i64::try_from(now).map_err(|_| ApiError::internal_message("timestamp exceeds i64 range"))
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
    use crate::domain::{
        policy::Visibility,
        store::{UserAccount, app_catalog},
    };

    #[test]
    fn retry_backoff_is_bounded() {
        assert_eq!(retry_delay_seconds(1), 5);
        assert_eq!(retry_delay_seconds(2), 10);
        assert_eq!(retry_delay_seconds(7), 300);
        assert_eq!(retry_delay_seconds(99), 300);
    }

    #[test]
    fn retry_policy_stops_at_max_attempts() {
        assert_eq!(next_retry_attempt(0).unwrap(), 1);
        assert!(!is_terminal_retry_attempt(MAX_JOB_ATTEMPTS - 1));
        assert!(is_terminal_retry_attempt(MAX_JOB_ATTEMPTS));
        assert!(is_terminal_retry_attempt(MAX_JOB_ATTEMPTS + 10));
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn repository_deletion_makes_claimed_outbox_job_obsolete() {
        let target = super::super::TestDatabaseTarget::required().unwrap();
        let store = MetadataStore::connect_fresh_for_tests(&target).unwrap();
        seed_outbox_repo(&store).await;

        let (claimed_tx, claimed_rx) = tokio::sync::oneshot::channel();
        let (deleted_tx, deleted_rx) = tokio::sync::oneshot::channel();
        let worker_store = store.clone();
        let worker = tokio::spawn(async move {
            let job = claim_next_ready_job(worker_store.db.as_ref(), "worker", 60)
                .await?
                .expect("the rebuild job should be ready");
            claimed_tx.send(job.id.clone()).unwrap();
            deleted_rx.await.unwrap();
            execute_outbox_job(worker_store.db.as_ref(), &job).await?;
            complete_outbox_job(worker_store.db.as_ref(), &job, "worker").await
        });

        let job_id = claimed_rx.await.unwrap();
        store
            .delete_repo("owner", "repo", "user_owner")
            .await
            .unwrap();
        assert!(
            entities::outbox_job::Entity::find_by_id(job_id)
                .one(store.db.as_ref())
                .await
                .unwrap()
                .is_none(),
            "repository deletion should cascade the claimed outbox job"
        );
        deleted_tx.send(()).unwrap();

        worker.await.unwrap().unwrap();
    }

    #[tokio::test]
    async fn live_repository_still_reports_a_stolen_outbox_lease() {
        let target = super::super::TestDatabaseTarget::required().unwrap();
        let store = MetadataStore::connect_fresh_for_tests(&target).unwrap();
        seed_outbox_repo(&store).await;
        let job = claim_next_ready_job(store.db.as_ref(), "worker", 60)
            .await
            .unwrap()
            .unwrap();
        entities::outbox_job::Entity::update_many()
            .filter(entities::outbox_job::Column::Id.eq(job.id.clone()))
            .col_expr(
                entities::outbox_job::Column::LeaseOwner,
                Expr::value(Some("other-worker".to_string())),
            )
            .exec(store.db.as_ref())
            .await
            .unwrap();

        let error = complete_outbox_job(store.db.as_ref(), &job, "worker")
            .await
            .unwrap_err();
        assert!(error.message.contains("lease was lost"));
        assert!(store.repository("owner", "repo").await.unwrap().is_some());
    }

    #[tokio::test]
    async fn claims_are_exclusive_and_expired_leases_are_reclaimed() {
        let target = super::super::TestDatabaseTarget::required().unwrap();
        let store = MetadataStore::connect_fresh_for_tests(&target).unwrap();
        seed_outbox_repo(&store).await;
        let barrier = Arc::new(tokio::sync::Barrier::new(2));
        let mut tasks = tokio::task::JoinSet::new();
        for worker in ["one", "two"] {
            let store = store.clone();
            let barrier = Arc::clone(&barrier);
            tasks.spawn(async move {
                barrier.wait().await;
                (
                    worker,
                    claim_next_ready_job(store.db.as_ref(), worker, 60)
                        .await
                        .unwrap(),
                )
            });
        }
        let mut claims = Vec::new();
        while let Some(result) = tasks.join_next().await {
            let (worker, claim) = result.unwrap();
            if let Some(claim) = claim {
                claims.push((worker, claim));
            }
        }
        assert_eq!(claims.len(), 1);
        let (stale_owner, stale) = claims.pop().unwrap();

        entities::outbox_job::Entity::update_many()
            .filter(entities::outbox_job::Column::Id.eq(stale.id.clone()))
            .col_expr(
                entities::outbox_job::Column::LeaseExpiresAtUnix,
                Expr::value(Some(0_i64)),
            )
            .exec(store.db.as_ref())
            .await
            .unwrap();
        let current = claim_next_ready_job(store.db.as_ref(), "replacement", 60)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(current.id, stale.id);
        assert!(
            complete_outbox_job(store.db.as_ref(), &stale, stale_owner)
                .await
                .is_err()
        );
    }

    #[tokio::test]
    async fn terminal_retry_marks_job_failed() {
        let target = super::super::TestDatabaseTarget::required().unwrap();
        let store = MetadataStore::connect_fresh_for_tests(&target).unwrap();
        seed_outbox_repo(&store).await;
        let mut job = claim_next_ready_job(store.db.as_ref(), "worker", 60)
            .await
            .unwrap()
            .unwrap();
        job.attempts = MAX_JOB_ATTEMPTS - 1;

        fail_outbox_job(store.db.as_ref(), &job, "worker", "failed".to_string())
            .await
            .unwrap();
        let row = entities::outbox_job::Entity::find_by_id(job.id)
            .one(store.db.as_ref())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(row.state, JOB_FAILED);
        assert_eq!(row.attempts, MAX_JOB_ATTEMPTS);
        assert!(row.completed_at_unix.is_some());
    }

    async fn seed_outbox_repo(store: &MetadataStore) {
        let owner = UserAccount {
            id: "user_owner".to_string(),
            handle: "owner".to_string(),
            email: "owner@example.com".to_string(),
            email_verified: true,
        };
        let mut catalog = app_catalog();
        let repo = catalog
            .create_repository(&owner, "repo", Visibility::Private)
            .unwrap()
            .clone();
        catalog.users.insert(owner.id.clone(), owner);
        store.seed_catalog_for_tests(catalog).unwrap();
        enqueue_projection_read_model_rebuild(
            store.db.as_ref(),
            &repo.record.id,
            repo.record.change_version,
        )
        .await
        .unwrap();
    }
}
