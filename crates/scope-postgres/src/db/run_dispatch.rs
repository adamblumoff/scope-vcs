use super::{
    DispatchClaim, RunStore, entities,
    run_attempt_persistence::{
        grant_by_ids, jobs_for_run, locked_job, locked_run, runner_by_id, save_job, save_run,
        save_runner,
    },
    runner_protocol_cutover::{
        DispatchCutover, dispatch_cutover, guard_claim, mark_canary_claimed,
    },
    runs::{DispatchOffer, unique_conflict, workflow_revision_for_run},
};
use crate::error::PostgresError;
use scope_domain::runs::{job::reconcile_run, run::RunJobState, runner::Runner};
use sea_orm::{
    ConnectionTrait, DatabaseBackend, DatabaseTransaction, EntityTrait, IntoActiveModel,
    QuerySelect, Statement, TransactionTrait,
};

struct DispatchCandidate {
    run_id: String,
    job_key: String,
}

async fn runner_has_capacity(
    tx: &DatabaseTransaction,
    runner: &Runner,
) -> Result<bool, PostgresError> {
    let active_attempts = tx
        .query_one(Statement::from_sql_and_values(
            DatabaseBackend::Postgres,
            "SELECT count(*) AS count
             FROM scope_run_attempts
             WHERE runner_id = $1 AND state IN ('leased', 'running')",
            [runner.id.clone().into()],
        ))
        .await
        .map_err(PostgresError::internal)?
        .ok_or_else(|| PostgresError::internal_message("runner capacity count is missing"))?
        .try_get::<i64>("", "count")
        .map_err(PostgresError::internal)?;
    Ok(active_attempts < i64::from(runner.max_concurrent_jobs.get()))
}

async fn dispatch_candidate(
    tx: &DatabaseTransaction,
    runner_id: &str,
    canary_run_id: Option<&str>,
) -> Result<Option<DispatchCandidate>, PostgresError> {
    const ELIGIBLE_JOB: &str = "
        SELECT job.run_id, job.job_key
        FROM scope_run_jobs job
        JOIN scope_runs run ON run.id = job.run_id
        JOIN scope_runner_grants runner_grant
          ON runner_grant.repo_id = run.repo_id
         AND runner_grant.runner_id = $1
         AND runner_grant.revoked_at_unix IS NULL
        WHERE job.state = 'queued'
          AND run.state IN ('queued', 'leased', 'running')
          AND run.cancellation_requested = FALSE
          AND (job.desired_runner_name IS NULL OR job.desired_runner_name = runner_grant.name)
        ORDER BY job.created_at_unix, job.run_id, job.job_key
        LIMIT 1";
    const ELIGIBLE_CANARY_JOB: &str = "
        SELECT job.run_id, job.job_key
        FROM scope_run_jobs job
        JOIN scope_runs run ON run.id = job.run_id
        JOIN scope_runner_grants runner_grant
          ON runner_grant.repo_id = run.repo_id
         AND runner_grant.runner_id = $1
         AND runner_grant.revoked_at_unix IS NULL
        WHERE job.state = 'queued'
          AND run.state IN ('queued', 'leased', 'running')
          AND run.cancellation_requested = FALSE
          AND (job.desired_runner_name IS NULL OR job.desired_runner_name = runner_grant.name)
          AND job.run_id = $2
        ORDER BY job.created_at_unix, job.run_id, job.job_key
        LIMIT 1";
    let statement = match canary_run_id {
        Some(run_id) => Statement::from_sql_and_values(
            DatabaseBackend::Postgres,
            ELIGIBLE_CANARY_JOB,
            [runner_id.into(), run_id.into()],
        ),
        None => Statement::from_sql_and_values(
            DatabaseBackend::Postgres,
            ELIGIBLE_JOB,
            [runner_id.into()],
        ),
    };
    tx.query_one(statement)
        .await
        .map_err(PostgresError::internal)?
        .map(|row| {
            Ok(DispatchCandidate {
                run_id: row.try_get("", "run_id").map_err(PostgresError::internal)?,
                job_key: row
                    .try_get("", "job_key")
                    .map_err(PostgresError::internal)?,
            })
        })
        .transpose()
}

impl RunStore {
    pub async fn next_dispatchable_job(
        &self,
        runner_id: &str,
    ) -> Result<Option<DispatchOffer>, PostgresError> {
        let tx = self.db.begin().await.map_err(PostgresError::internal)?;
        // Polling is advisory. Do not lock the runner row here: claims and attempt
        // transitions lock jobs before runners, and reversing that order in poll
        // would allow a poll and claim to deadlock. The claim path below performs
        // the authoritative capacity check while holding the runner lock.
        let runner = entities::runner::Entity::find_by_id(runner_id.to_string())
            .one(&tx)
            .await
            .map_err(PostgresError::internal)?
            .ok_or_else(|| PostgresError::not_found("runner not found"))?
            .try_into_domain()?;
        if !runner.supports_dispatch() {
            tx.commit().await.map_err(PostgresError::internal)?;
            return Ok(None);
        }
        if !runner_has_capacity(&tx, &runner).await? {
            tx.commit().await.map_err(PostgresError::internal)?;
            return Ok(None);
        }
        let dispatch = dispatch_cutover(&tx, runner_id, runner.protocol_version).await?;
        if matches!(dispatch, DispatchCutover::None) {
            tx.commit().await.map_err(PostgresError::internal)?;
            return Ok(None);
        }

        let canary_run_id = match &dispatch {
            DispatchCutover::Canary(run_id) => Some(run_id.as_str()),
            DispatchCutover::General => None,
            DispatchCutover::None => unreachable!("none dispatch returned above"),
        };
        let Some(candidate) = dispatch_candidate(&tx, runner_id, canary_run_id).await? else {
            tx.commit().await.map_err(PostgresError::internal)?;
            return Ok(None);
        };
        let job = locked_job(&tx, &candidate.run_id, &candidate.job_key).await?;
        let run = locked_run(&tx, &candidate.run_id).await?;
        let grant = entities::runner_grant::Entity::find_by_id((
            run.workflow.repository_id().to_string(),
            runner_id.to_string(),
        ))
        .lock_exclusive()
        .one(&tx)
        .await
        .map_err(PostgresError::internal)?;
        let still_eligible = job.state == RunJobState::Queued
            && !run.cancellation_requested
            && !run.state.is_terminal()
            && grant.is_some_and(|grant| {
                grant.revoked_at_unix.is_none()
                    && job.desired_runner.matches_name(grant.name.as_str())
            })
            && canary_run_id.is_none_or(|run_id| run.id == run_id);
        tx.commit().await.map_err(PostgresError::internal)?;
        Ok(still_eligible.then_some(DispatchOffer { run, job }))
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn claim_job(
        &self,
        run_id: &str,
        job_key: &str,
        runner_id: &str,
        attempt_id: &str,
        token_hash: &str,
        now_unix: u64,
        lease_expires_at_unix: u64,
    ) -> Result<DispatchClaim, PostgresError> {
        let tx = self.db.begin().await.map_err(PostgresError::internal)?;
        guard_claim(&tx, runner_id, run_id).await?;
        let run_snapshot = entities::run::Entity::find_by_id(run_id.to_string())
            .one(&tx)
            .await
            .map_err(PostgresError::internal)?
            .ok_or_else(|| PostgresError::not_found("run not found"))?
            .try_into_domain()?;
        let mut job = locked_job(&tx, run_id, job_key).await?;
        let mut runner = runner_by_id(&tx, runner_id).await?;
        if !runner_has_capacity(&tx, &runner).await? {
            return Err(PostgresError::resource_exhausted(format!(
                "runner has reached its capacity of {} concurrent job(s)",
                runner.max_concurrent_jobs.get()
            )));
        }
        let grant = grant_by_ids(&tx, run_snapshot.workflow.repository_id(), runner_id).await?;
        let workflow_revision = workflow_revision_for_run(&tx, &run_snapshot).await?;
        let definition = workflow_revision
            .definition()
            .job(&job.key)
            .ok_or_else(|| PostgresError::internal_message("run job definition is missing"))?;
        let (attempt, steps) = job
            .claim(
                &run_snapshot,
                definition,
                &runner,
                &grant,
                attempt_id,
                token_hash,
                now_unix,
                lease_expires_at_unix,
            )
            .map_err(PostgresError::from)?;
        runner.record_seen(now_unix).map_err(PostgresError::from)?;

        entities::run_attempt::Entity::insert(
            entities::run_attempt::Model::from_domain(&attempt)?.into_active_model(),
        )
        .exec(&tx)
        .await
        .map_err(|error| {
            unique_conflict(error, "run attempt id or token hash is already in use")
        })?;
        entities::run_attempt_step::Entity::insert_many(
            steps
                .iter()
                .map(entities::run_attempt_step::Model::from_domain)
                .collect::<Result<Vec<_>, _>>()?
                .into_iter()
                .map(IntoActiveModel::into_active_model),
        )
        .exec(&tx)
        .await
        .map_err(PostgresError::internal)?;
        save_job(&tx, &job).await?;
        let mut run = locked_run(&tx, run_id).await?;
        if run.cancellation_requested || run.state.is_terminal() {
            return Err(PostgresError::conflict("run is no longer dispatchable"));
        }
        let mut jobs = jobs_for_run(&tx, run_id).await?;
        if let Some(stored) = jobs.iter_mut().find(|stored| stored.key == job.key) {
            *stored = job.clone();
        }
        reconcile_run(&mut run, &mut jobs, &workflow_revision, now_unix)
            .map_err(PostgresError::from)?;
        save_run(&tx, &run).await?;
        save_runner(&tx, &runner).await?;
        let canary_phase = mark_canary_claimed(&tx, runner_id, run_id, now_unix).await?;
        tx.commit().await.map_err(PostgresError::internal)?;
        Ok(DispatchClaim {
            run,
            job,
            attempt,
            steps,
            workflow_revision,
            canary_phase,
        })
    }
}
