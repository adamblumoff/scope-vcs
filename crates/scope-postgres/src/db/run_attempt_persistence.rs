use super::entities;
use crate::error::PostgresError;
use scope_domain::runs::{
    job::RunJob,
    run::{Run, RunAttempt, RunAttemptStep},
    runner::{Runner, RunnerGrant},
};
use sea_orm::{
    ActiveModelTrait, ColumnTrait, DatabaseTransaction, EntityTrait, IntoActiveModel, QueryFilter,
    QueryOrder, QuerySelect,
};

pub(super) async fn locked_run(
    tx: &DatabaseTransaction,
    run_id: &str,
) -> Result<Run, PostgresError> {
    entities::run::Entity::find_by_id(run_id.to_string())
        .lock_exclusive()
        .one(tx)
        .await
        .map_err(PostgresError::internal)?
        .ok_or_else(|| PostgresError::not_found("run not found"))?
        .try_into_domain()
}

pub(super) async fn locked_attempt_context(
    tx: &DatabaseTransaction,
    attempt_id: &str,
) -> Result<(Run, RunJob, RunAttempt, Vec<RunAttemptStep>), PostgresError> {
    let target = entities::run_attempt::Entity::find_by_id(attempt_id.to_string())
        .one(tx)
        .await
        .map_err(PostgresError::internal)?
        .ok_or_else(|| PostgresError::not_found("run attempt not found"))?;
    let run = entities::run::Entity::find_by_id(target.run_id.clone())
        .one(tx)
        .await
        .map_err(PostgresError::internal)?
        .ok_or_else(|| PostgresError::not_found("run not found"))?
        .try_into_domain()?;
    let job = locked_job(tx, &target.run_id, &target.job_key).await?;
    let attempt = entities::run_attempt::Entity::find_by_id(attempt_id.to_string())
        .lock_exclusive()
        .one(tx)
        .await
        .map_err(PostgresError::internal)?
        .ok_or_else(|| PostgresError::not_found("run attempt not found"))?
        .try_into_domain()?;
    let steps = locked_attempt_steps(tx, attempt_id).await?;
    Ok((run, job, attempt, steps))
}

pub(super) async fn locked_heartbeat_context(
    tx: &DatabaseTransaction,
    attempt_id: &str,
) -> Result<(Run, RunJob, RunAttempt), PostgresError> {
    let target = entities::run_attempt::Entity::find_by_id(attempt_id.to_string())
        .one(tx)
        .await
        .map_err(PostgresError::internal)?
        .ok_or_else(|| PostgresError::not_found("run attempt not found"))?;
    let job = locked_job(tx, &target.run_id, &target.job_key).await?;
    // Cancellation takes the job lock before mutating the parent. Reading the parent only after
    // acquiring that job lock observes any cancellation that committed while heartbeat waited,
    // while keeping ordinary runner traffic free of an aggregate-wide parent lock.
    let run = entities::run::Entity::find_by_id(target.run_id.clone())
        .one(tx)
        .await
        .map_err(PostgresError::internal)?
        .ok_or_else(|| PostgresError::not_found("run not found"))?
        .try_into_domain()?;
    let attempt = entities::run_attempt::Entity::find_by_id(attempt_id.to_string())
        .lock_exclusive()
        .one(tx)
        .await
        .map_err(PostgresError::internal)?
        .ok_or_else(|| PostgresError::not_found("run attempt not found"))?
        .try_into_domain()?;
    Ok((run, job, attempt))
}

pub(super) async fn locked_job(
    tx: &DatabaseTransaction,
    run_id: &str,
    job_key: &str,
) -> Result<RunJob, PostgresError> {
    entities::run_job::Entity::find_by_id((run_id.to_string(), job_key.to_string()))
        .lock_exclusive()
        .one(tx)
        .await
        .map_err(PostgresError::internal)?
        .ok_or_else(|| PostgresError::not_found("run job not found"))?
        .try_into_domain()
}

pub(super) async fn locked_jobs(
    tx: &DatabaseTransaction,
    run_id: &str,
) -> Result<Vec<RunJob>, PostgresError> {
    entities::run_job::Entity::find()
        .filter(entities::run_job::Column::RunId.eq(run_id))
        .order_by_asc(entities::run_job::Column::JobKey)
        .lock_exclusive()
        .all(tx)
        .await
        .map_err(PostgresError::internal)?
        .into_iter()
        .map(entities::run_job::Model::try_into_domain)
        .collect()
}

pub(super) async fn jobs_for_run(
    tx: &DatabaseTransaction,
    run_id: &str,
) -> Result<Vec<RunJob>, PostgresError> {
    entities::run_job::Entity::find()
        .filter(entities::run_job::Column::RunId.eq(run_id))
        .order_by_asc(entities::run_job::Column::JobKey)
        .all(tx)
        .await
        .map_err(PostgresError::internal)?
        .into_iter()
        .map(entities::run_job::Model::try_into_domain)
        .collect()
}

pub(super) async fn attempt_target(
    tx: &DatabaseTransaction,
    attempt_id: &str,
) -> Result<(String, String), PostgresError> {
    let attempt = entities::run_attempt::Entity::find_by_id(attempt_id.to_string())
        .one(tx)
        .await
        .map_err(PostgresError::internal)?
        .ok_or_else(|| PostgresError::not_found("run attempt not found"))?;
    Ok((attempt.run_id, attempt.runner_id))
}

pub(super) async fn locked_attempt_steps(
    tx: &DatabaseTransaction,
    attempt_id: &str,
) -> Result<Vec<RunAttemptStep>, PostgresError> {
    entities::run_attempt_step::Entity::find()
        .filter(entities::run_attempt_step::Column::AttemptId.eq(attempt_id))
        .order_by_asc(entities::run_attempt_step::Column::StepIndex)
        .lock_exclusive()
        .all(tx)
        .await
        .map_err(PostgresError::internal)?
        .into_iter()
        .map(entities::run_attempt_step::Model::try_into_domain)
        .collect()
}

pub(super) async fn runner_by_id(
    tx: &DatabaseTransaction,
    runner_id: &str,
) -> Result<Runner, PostgresError> {
    entities::runner::Entity::find_by_id(runner_id.to_string())
        .lock_exclusive()
        .one(tx)
        .await
        .map_err(PostgresError::internal)?
        .ok_or_else(|| PostgresError::not_found("runner not found"))?
        .try_into_domain()
}

pub(super) async fn grant_by_ids(
    tx: &DatabaseTransaction,
    repository_id: &str,
    runner_id: &str,
) -> Result<RunnerGrant, PostgresError> {
    entities::runner_grant::Entity::find_by_id((repository_id.to_string(), runner_id.to_string()))
        .lock_exclusive()
        .one(tx)
        .await
        .map_err(PostgresError::internal)?
        .ok_or_else(|| {
            PostgresError::permission_denied("runner is not attached to the repository")
        })?
        .try_into_domain()
}

pub(super) async fn ensure_runner_authorized(
    tx: &DatabaseTransaction,
    run: &Run,
    attempt: &RunAttempt,
) -> Result<Runner, PostgresError> {
    let runner = runner_by_id(tx, &attempt.runner_id).await?;
    let grant = grant_by_ids(tx, run.workflow.repository_id(), &attempt.runner_id).await?;
    if !runner.enabled || !grant.is_active() {
        return Err(PostgresError::permission_denied(
            "runner or repository grant is revoked",
        ));
    }
    Ok(runner)
}

pub(super) async fn save_run(tx: &DatabaseTransaction, run: &Run) -> Result<(), PostgresError> {
    entities::run::Entity::update(
        entities::run::Model::from_domain(run)?
            .into_active_model()
            .reset_all(),
    )
    .exec(tx)
    .await
    .map_err(PostgresError::internal)?;
    Ok(())
}

pub(super) async fn save_job(tx: &DatabaseTransaction, job: &RunJob) -> Result<(), PostgresError> {
    entities::run_job::Entity::update(
        entities::run_job::Model::from_domain(job)?
            .into_active_model()
            .reset_all(),
    )
    .exec(tx)
    .await
    .map_err(PostgresError::internal)?;
    Ok(())
}

pub(super) async fn save_jobs(
    tx: &DatabaseTransaction,
    jobs: &[RunJob],
) -> Result<(), PostgresError> {
    for job in jobs {
        save_job(tx, job).await?;
    }
    Ok(())
}

pub(super) async fn save_attempt(
    tx: &DatabaseTransaction,
    attempt: &RunAttempt,
) -> Result<(), PostgresError> {
    entities::run_attempt::Entity::update(
        entities::run_attempt::Model::from_domain(attempt)?
            .into_active_model()
            .reset_all(),
    )
    .exec(tx)
    .await
    .map_err(PostgresError::internal)?;
    Ok(())
}

pub(super) async fn save_attempt_steps(
    tx: &DatabaseTransaction,
    steps: &[RunAttemptStep],
) -> Result<(), PostgresError> {
    for step in steps {
        entities::run_attempt_step::Entity::update(
            entities::run_attempt_step::Model::from_domain(step)?
                .into_active_model()
                .reset_all(),
        )
        .exec(tx)
        .await
        .map_err(PostgresError::internal)?;
    }
    Ok(())
}

pub(super) async fn save_runner(
    tx: &DatabaseTransaction,
    runner: &Runner,
) -> Result<(), PostgresError> {
    entities::runner::Entity::update(
        entities::runner::Model::from_domain(runner)?
            .into_active_model()
            .reset_all(),
    )
    .exec(tx)
    .await
    .map_err(PostgresError::internal)?;
    Ok(())
}
