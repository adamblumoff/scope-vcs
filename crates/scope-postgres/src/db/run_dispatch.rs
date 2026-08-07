use super::{
    DispatchClaim, RunStore, entities,
    run_attempt_persistence::{
        grant_by_ids, jobs_for_run, locked_job, runner_by_id, save_job, save_run, save_runner,
    },
    runner_protocol_cutover::{
        DispatchCutover, dispatch_cutover, guard_claim, mark_canary_claimed,
    },
    runs::{DispatchOffer, unique_conflict, workflow_revision_for_run},
};
use crate::error::PostgresError;
use scope_domain::runs::job::reconcile_run;
use sea_orm::{
    ColumnTrait, EntityTrait, IntoActiveModel, QueryFilter, QueryOrder, TransactionTrait,
};

impl RunStore {
    pub async fn next_dispatchable_job(
        &self,
        runner_id: &str,
    ) -> Result<Option<DispatchOffer>, PostgresError> {
        let tx = self.db.begin().await.map_err(PostgresError::internal)?;
        let runner = entities::runner::Entity::find_by_id(runner_id.to_string())
            .one(&tx)
            .await
            .map_err(PostgresError::internal)?
            .ok_or_else(|| PostgresError::not_found("runner not found"))?
            .try_into_domain()?;
        if !runner.supports_dispatch() {
            return Ok(None);
        }
        let dispatch = dispatch_cutover(&tx, runner_id, runner.protocol_version).await?;
        if matches!(dispatch, DispatchCutover::None) {
            tx.commit().await.map_err(PostgresError::internal)?;
            return Ok(None);
        }

        let grants = entities::runner_grant::Entity::find()
            .filter(entities::runner_grant::Column::RunnerId.eq(runner_id))
            .filter(entities::runner_grant::Column::RevokedAtUnix.is_null())
            .all(&tx)
            .await
            .map_err(PostgresError::internal)?;
        let grants = grants
            .into_iter()
            .map(|grant| (grant.repo_id, grant.name))
            .collect::<Vec<_>>();
        if grants.is_empty() {
            return Ok(None);
        }
        let jobs = entities::run_job::Entity::find()
            .filter(entities::run_job::Column::State.eq("queued"))
            .order_by_asc(entities::run_job::Column::CreatedAtUnix)
            .order_by_asc(entities::run_job::Column::RunId)
            .order_by_asc(entities::run_job::Column::JobKey)
            .all(&tx)
            .await
            .map_err(PostgresError::internal)?;
        for job_model in jobs {
            if let DispatchCutover::Canary(ref run_id) = dispatch
                && &job_model.run_id != run_id
            {
                continue;
            }
            let job = job_model.try_into_domain()?;
            let run = entities::run::Entity::find_by_id(job.run_id.clone())
                .one(&tx)
                .await
                .map_err(PostgresError::internal)?
                .ok_or_else(|| PostgresError::internal_message("run job parent is missing"))?
                .try_into_domain()?;
            if !run.cancellation_requested
                && !run.state.is_terminal()
                && grants.iter().any(|(repo_id, name)| {
                    repo_id == run.workflow.repository_id()
                        && job.desired_runner.matches_name(name.as_str())
                })
            {
                tx.commit().await.map_err(PostgresError::internal)?;
                return Ok(Some(DispatchOffer { run, job }));
            }
        }
        tx.commit().await.map_err(PostgresError::internal)?;
        Ok(None)
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
        let mut run = entities::run::Entity::find_by_id(run_id.to_string())
            .one(&tx)
            .await
            .map_err(PostgresError::internal)?
            .ok_or_else(|| PostgresError::not_found("run not found"))?
            .try_into_domain()?;
        let mut job = locked_job(&tx, run_id, job_key).await?;
        let mut runner = runner_by_id(&tx, runner_id).await?;
        let grant = grant_by_ids(&tx, run.workflow.repository_id(), runner_id).await?;
        let workflow_revision = workflow_revision_for_run(&tx, &run).await?;
        let definition = workflow_revision
            .definition()
            .job(&job.key)
            .ok_or_else(|| PostgresError::internal_message("run job definition is missing"))?;
        let (attempt, steps) = job
            .claim(
                &run,
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
