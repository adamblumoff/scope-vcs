use super::{
    RunStore, entities,
    git_segments::insert_git_segment_references,
    object_references::insert_object_reference,
    run_attempt_persistence::{
        attempt_run_id, jobs_for_run, locked_attempt_steps, locked_heartbeat_context, locked_jobs,
        locked_run, save_attempt, save_attempt_steps, save_jobs, save_run,
    },
};
use crate::error::PostgresError;
use scope_domain::runs::{
    attempt::RunAttempt,
    job::{RunJob, create_run_jobs, reconcile_run, request_run_cancellation, retry_run},
    run::Run,
    step::{AttemptConclusion, RunAttemptStep},
    workflow::revision::WorkflowRevision,
};
use sea_orm::{
    ColumnTrait, DatabaseTransaction, DbErr, EntityTrait, IntoActiveModel, QueryFilter, QueryOrder,
    QuerySelect, TransactionTrait, TryInsertResult, sea_query::OnConflict,
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DispatchClaim {
    pub run: Run,
    pub job: RunJob,
    pub attempt: RunAttempt,
    pub steps: Vec<RunAttemptStep>,
    pub workflow_revision: WorkflowRevision,
}

#[cfg(any(
    test,
    feature = "test-support",
    feature = "local-dev",
    feature = "smoke-seed"
))]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DispatchOffer {
    pub run: Run,
    pub job: RunJob,
}

impl RunStore {
    pub async fn enqueue_run(
        &self,
        run: Run,
        revision: WorkflowRevision,
    ) -> Result<EnqueueRunResult, PostgresError> {
        let tx = self.db.begin().await.map_err(PostgresError::internal)?;
        let stored = enqueue_run_in_transaction(&tx, run, revision).await?;
        tx.commit().await.map_err(PostgresError::internal)?;
        Ok(stored)
    }

    pub async fn heartbeat_attempt(
        &self,
        attempt_id: &str,
        token_hash: &str,
        now_unix: u64,
        lease_expires_at_unix: u64,
    ) -> Result<bool, PostgresError> {
        let tx = self.db.begin().await.map_err(PostgresError::internal)?;
        let (run, job, mut attempt) = locked_heartbeat_context(&tx, attempt_id).await?;
        let cancellation_requested = attempt
            .heartbeat(&run, &job, token_hash, now_unix, lease_expires_at_unix)
            .map_err(PostgresError::from)?;
        save_attempt(&tx, &attempt).await?;
        tx.commit().await.map_err(PostgresError::internal)?;
        Ok(cancellation_requested)
    }

    pub async fn complete_attempt(
        &self,
        attempt_id: &str,
        token_hash: &str,
        conclusion: AttemptConclusion,
        logs_truncated: bool,
        now_unix: u64,
    ) -> Result<DispatchClaim, PostgresError> {
        self.mutate_attempt(attempt_id, |run, job, attempt, steps| {
            attempt.complete(
                run,
                job,
                steps,
                token_hash,
                conclusion,
                logs_truncated,
                now_unix,
            )
        })
        .await
    }

    pub async fn abandon_attempt(
        &self,
        attempt_id: &str,
        token_hash: &str,
        now_unix: u64,
    ) -> Result<DispatchClaim, PostgresError> {
        self.mutate_attempt(attempt_id, |run, job, attempt, steps| {
            attempt.abandon(run, job, steps, token_hash, now_unix)
        })
        .await
    }

    pub async fn confirm_provider_cancellation(
        &self,
        attempt_id: &str,
        now_unix: u64,
    ) -> Result<DispatchClaim, PostgresError> {
        self.mutate_attempt(attempt_id, |run, job, attempt, steps| {
            attempt.confirm_provider_cancellation(run, job, steps, now_unix)
        })
        .await
    }

    pub async fn expire_attempt(
        &self,
        attempt_id: &str,
        now_unix: u64,
    ) -> Result<DispatchClaim, PostgresError> {
        let tx = self.db.begin().await.map_err(PostgresError::internal)?;
        let guard_run_id = attempt_run_id(&tx, attempt_id).await?;
        let mut jobs = locked_jobs(&tx, &guard_run_id).await?;
        let mut run = locked_run(&tx, &guard_run_id).await?;
        let mut attempt = entities::run_attempt::Entity::find_by_id(attempt_id.to_string())
            .lock_exclusive()
            .one(&tx)
            .await
            .map_err(PostgresError::internal)?
            .ok_or_else(|| PostgresError::not_found("run attempt not found"))?
            .try_into_domain()?;
        let mut steps = locked_attempt_steps(&tx, attempt_id).await?;
        let job = jobs
            .iter_mut()
            .find(|job| job.key == attempt.job_key)
            .ok_or_else(|| PostgresError::internal_message("run attempt job is missing"))?;
        attempt
            .expire(&run, job, &mut steps, now_unix)
            .map_err(PostgresError::from)?;
        let workflow_revision = workflow_revision_for_run(&tx, &run).await?;
        reconcile_run(&mut run, &mut jobs, &workflow_revision, now_unix)
            .map_err(PostgresError::from)?;
        save_attempt(&tx, &attempt).await?;
        save_attempt_steps(&tx, &steps).await?;
        save_jobs(&tx, &jobs).await?;
        save_run(&tx, &run).await?;
        tx.commit().await.map_err(PostgresError::internal)?;
        let job = jobs
            .into_iter()
            .find(|job| job.key == attempt.job_key)
            .ok_or_else(|| PostgresError::internal_message("run attempt job is missing"))?;
        Ok(DispatchClaim {
            run,
            job,
            attempt,
            steps,
            workflow_revision,
        })
    }

    pub async fn expired_attempt_ids(
        &self,
        now_unix: u64,
        limit: u64,
    ) -> Result<Vec<String>, PostgresError> {
        let now_unix = entities::u64_to_i64(now_unix, "attempt recovery time")?;
        let maximum_age_cutoff = now_unix
            .saturating_sub(scope_domain::runs::attempt::MAX_RUN_ATTEMPT_AGE_SECONDS as i64);
        Ok(entities::run_attempt::Entity::find()
            .filter(
                entities::run_attempt::Column::State
                    .is_in(["dispatching".to_string(), "running".to_string()]),
            )
            .filter(
                entities::run_attempt::Column::LeaseExpiresAtUnix
                    .lte(now_unix)
                    .or(entities::run_attempt::Column::CreatedAtUnix.lte(maximum_age_cutoff)),
            )
            .order_by_asc(entities::run_attempt::Column::LeaseExpiresAtUnix)
            .order_by_asc(entities::run_attempt::Column::Id)
            .limit(limit)
            .all(self.db.as_ref())
            .await
            .map_err(PostgresError::internal)?
            .into_iter()
            .map(|attempt| attempt.id)
            .collect())
    }

    pub async fn request_run_cancellation(
        &self,
        run_id: &str,
        now_unix: u64,
    ) -> Result<Run, PostgresError> {
        let tx = self.db.begin().await.map_err(PostgresError::internal)?;
        let mut jobs = locked_jobs(&tx, run_id).await?;
        let mut run = locked_run(&tx, run_id).await?;
        request_run_cancellation(&mut run, &mut jobs, now_unix).map_err(PostgresError::from)?;
        save_jobs(&tx, &jobs).await?;
        save_run(&tx, &run).await?;
        tx.commit().await.map_err(PostgresError::internal)?;
        Ok(run)
    }

    pub async fn retry_run(&self, run_id: &str, now_unix: u64) -> Result<Run, PostgresError> {
        let tx = self.db.begin().await.map_err(PostgresError::internal)?;
        let mut jobs = locked_jobs(&tx, run_id).await?;
        let mut run = locked_run(&tx, run_id).await?;
        let revision = workflow_revision_for_run(&tx, &run).await?;
        retry_run(&mut run, &mut jobs, &revision, now_unix).map_err(PostgresError::from)?;
        save_jobs(&tx, &jobs).await?;
        save_run(&tx, &run).await?;
        tx.commit().await.map_err(PostgresError::internal)?;
        Ok(run)
    }

    pub async fn run(&self, run_id: &str) -> Result<Option<Run>, PostgresError> {
        entities::run::Entity::find_by_id(run_id.to_string())
            .one(self.db.as_ref())
            .await
            .map_err(PostgresError::internal)?
            .map(entities::run::Model::try_into_domain)
            .transpose()
    }
}

pub struct EnqueueRunResult {
    pub run: Run,
    pub inserted: bool,
}

pub(super) async fn enqueue_run_in_transaction(
    tx: &DatabaseTransaction,
    run: Run,
    revision: WorkflowRevision,
) -> Result<EnqueueRunResult, PostgresError> {
    run.validate_workflow_revision(&revision)
        .map_err(PostgresError::from)?;
    let requested_jobs = create_run_jobs(&run, &revision).map_err(PostgresError::from)?;
    save_workflow_revision(tx, &revision, run.created_at_unix).await?;
    let model = entities::run::ActiveModel::from_domain(&run)?;
    let result = entities::run::Entity::insert(model)
        .on_conflict(OnConflict::new().do_nothing().to_owned())
        .do_nothing()
        .exec(tx)
        .await
        .map_err(PostgresError::internal)?;
    let inserted = matches!(result, TryInsertResult::Inserted(_));
    if !inserted {
        let stored = entities::run::Entity::find()
            .filter(entities::run::Column::RepoId.eq(run.workflow.repository_id().to_string()))
            .filter(entities::run::Column::IdempotencyKey.eq(run.idempotency_key.clone()))
            .one(tx)
            .await
            .map_err(PostgresError::internal)?
            .ok_or_else(|| {
                PostgresError::conflict("run id is already used by another idempotency key")
            })?
            .try_into_domain()?;
        let stored_jobs = jobs_for_run(tx, &stored.id).await?;
        if !stored.has_same_enqueue_request_identity(&run) || stored_jobs != requested_jobs {
            return Err(PostgresError::conflict(
                "run idempotency key is already used by a different enqueue request",
            ));
        }
        return Ok(EnqueueRunResult {
            run: stored,
            inserted: false,
        });
    }
    entities::run_job::Entity::insert_many(
        requested_jobs
            .iter()
            .map(entities::run_job::Model::from_domain)
            .collect::<Result<Vec<_>, _>>()?
            .into_iter()
            .map(IntoActiveModel::into_active_model),
    )
    .exec(tx)
    .await
    .map_err(PostgresError::internal)?;
    for object in run.source.retained_objects() {
        insert_object_reference(tx, "run_source", &run.id, object).await?;
    }
    insert_git_segment_references(
        tx,
        "run_source",
        &run.id,
        run.source.retained_git_segments(),
    )
    .await?;
    Ok(EnqueueRunResult {
        run,
        inserted: true,
    })
}

async fn save_workflow_revision(
    tx: &DatabaseTransaction,
    revision: &WorkflowRevision,
    created_at_unix: u64,
) -> Result<(), PostgresError> {
    entities::workflow_revision::Entity::insert(
        entities::workflow_revision::Model::from_domain(revision, created_at_unix)?
            .into_active_model(),
    )
    .on_conflict(
        OnConflict::column(entities::workflow_revision::Column::Digest)
            .do_nothing()
            .to_owned(),
    )
    .do_nothing()
    .exec(tx)
    .await
    .map_err(PostgresError::internal)?;
    let persisted = entities::workflow_revision::Entity::find_by_id(revision.digest().to_string())
        .one(tx)
        .await
        .map_err(PostgresError::internal)?
        .ok_or_else(|| PostgresError::internal_message("workflow revision was not stored"))?;
    let persisted = persisted.try_into_domain(revision.workflow().clone())?;
    if &persisted != revision {
        return Err(PostgresError::conflict(
            "workflow revision digest is already used by a different definition",
        ));
    }
    Ok(())
}

pub(super) async fn workflow_revision_for_run(
    tx: &DatabaseTransaction,
    run: &Run,
) -> Result<WorkflowRevision, PostgresError> {
    entities::workflow_revision::Entity::find_by_id(run.workflow_revision_digest.clone())
        .one(tx)
        .await
        .map_err(PostgresError::internal)?
        .ok_or_else(|| PostgresError::internal_message("run workflow revision is missing"))?
        .try_into_domain(run.workflow.clone())
}

pub(super) fn unique_conflict(error: DbErr, message: &str) -> PostgresError {
    if matches!(
        error.sql_err(),
        Some(sea_orm::SqlErr::UniqueConstraintViolation(_))
    ) {
        PostgresError::conflict(message)
    } else {
        PostgresError::internal(error)
    }
}
