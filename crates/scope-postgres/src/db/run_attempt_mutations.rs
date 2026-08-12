use super::{DispatchClaim, RunStore, entities};
use crate::error::PostgresError;
use scope_domain::runs::{
    job::{RunJob, reconcile_run},
    run::{Run, RunAttempt, RunAttemptStep},
};
use sea_orm::{EntityTrait, QuerySelect, TransactionTrait};

impl RunStore {
    pub async fn claim_runtime(
        &self,
        attempt_id: &str,
        bootstrap_token_hash: &str,
        attempt_token_hash: &str,
        now_unix: u64,
        lease_expires_at_unix: u64,
    ) -> Result<DispatchClaim, PostgresError> {
        let tx = self.db.begin().await.map_err(PostgresError::internal)?;
        let (run, job, mut attempt, steps) =
            super::run_attempt_persistence::locked_attempt_context(&tx, attempt_id).await?;
        attempt
            .claim_runtime(
                &job,
                bootstrap_token_hash,
                attempt_token_hash,
                now_unix,
                lease_expires_at_unix,
            )
            .map_err(PostgresError::from)?;
        super::run_attempt_persistence::save_attempt(&tx, &attempt).await?;
        let workflow_revision = super::runs::workflow_revision_for_run(&tx, &run).await?;
        tx.commit().await.map_err(PostgresError::internal)?;
        Ok(DispatchClaim {
            run,
            job,
            attempt,
            steps,
            workflow_revision,
        })
    }

    pub async fn authenticate_attempt(
        &self,
        attempt_id: &str,
        token_hash: &str,
        now_unix: u64,
    ) -> Result<DispatchClaim, PostgresError> {
        let tx = self.db.begin().await.map_err(PostgresError::internal)?;
        let (run, job, attempt, steps) =
            super::run_attempt_persistence::locked_attempt_context(&tx, attempt_id).await?;
        attempt
            .authenticate_access(&job, token_hash, now_unix)
            .map_err(PostgresError::from)?;
        let workflow_revision = super::runs::workflow_revision_for_run(&tx, &run).await?;
        tx.commit().await.map_err(PostgresError::internal)?;
        Ok(DispatchClaim {
            run,
            job,
            attempt,
            steps,
            workflow_revision,
        })
    }

    pub async fn authenticate_attempt_cache(
        &self,
        attempt_id: &str,
        token_hash: &str,
        now_unix: u64,
    ) -> Result<DispatchClaim, PostgresError> {
        let tx = self.db.begin().await.map_err(PostgresError::internal)?;
        let (run, job, attempt, steps) =
            super::run_attempt_persistence::locked_attempt_context(&tx, attempt_id).await?;
        attempt
            .authenticate_cache_observation_report(&job, token_hash, now_unix)
            .map_err(PostgresError::from)?;
        let workflow_revision = super::runs::workflow_revision_for_run(&tx, &run).await?;
        tx.commit().await.map_err(PostgresError::internal)?;
        Ok(DispatchClaim {
            run,
            job,
            attempt,
            steps,
            workflow_revision,
        })
    }

    pub(super) async fn mutate_attempt(
        &self,
        attempt_id: &str,
        mutate: impl FnOnce(
            &Run,
            &mut RunJob,
            &mut RunAttempt,
            &mut [RunAttemptStep],
        ) -> Result<(), scope_domain::error::DomainError>,
    ) -> Result<DispatchClaim, PostgresError> {
        let tx = self.db.begin().await.map_err(PostgresError::internal)?;
        let guard_run_id = super::run_attempt_persistence::attempt_run_id(&tx, attempt_id).await?;
        let mut jobs = super::run_attempt_persistence::locked_jobs(&tx, &guard_run_id).await?;
        let mut run = super::run_attempt_persistence::locked_run(&tx, &guard_run_id).await?;
        let mut attempt = entities::run_attempt::Entity::find_by_id(attempt_id.to_string())
            .lock_exclusive()
            .one(&tx)
            .await
            .map_err(PostgresError::internal)?
            .ok_or_else(|| PostgresError::not_found("run attempt not found"))?
            .try_into_domain()?;
        let mut steps =
            super::run_attempt_persistence::locked_attempt_steps(&tx, attempt_id).await?;
        let job = jobs
            .iter_mut()
            .find(|job| job.key == attempt.job_key)
            .ok_or_else(|| PostgresError::internal_message("run attempt job is missing"))?;
        mutate(&run, job, &mut attempt, &mut steps).map_err(PostgresError::from)?;
        let workflow_revision = super::runs::workflow_revision_for_run(&tx, &run).await?;
        let transition_time = attempt.completed_at_unix.unwrap_or(job.updated_at_unix);
        reconcile_run(&mut run, &mut jobs, &workflow_revision, transition_time)
            .map_err(PostgresError::from)?;
        super::run_attempt_persistence::save_attempt(&tx, &attempt).await?;
        super::run_attempt_persistence::save_attempt_steps(&tx, &steps).await?;
        super::run_attempt_persistence::save_jobs(&tx, &jobs).await?;
        super::run_attempt_persistence::save_run(&tx, &run).await?;
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

    pub(super) async fn mutate_active_attempt(
        &self,
        attempt_id: &str,
        mutate: impl FnOnce(
            &Run,
            &mut RunJob,
            &mut RunAttempt,
            &mut [RunAttemptStep],
        ) -> Result<(), scope_domain::error::DomainError>,
    ) -> Result<DispatchClaim, PostgresError> {
        let tx = self.db.begin().await.map_err(PostgresError::internal)?;
        let (run_snapshot, mut job, mut attempt, mut steps) =
            super::run_attempt_persistence::locked_attempt_context(&tx, attempt_id).await?;
        let mut run = super::run_attempt_persistence::locked_run(&tx, &run_snapshot.id).await?;
        mutate(&run, &mut job, &mut attempt, &mut steps).map_err(PostgresError::from)?;
        super::run_attempt_persistence::save_job(&tx, &job).await?;
        super::run_attempt_persistence::save_attempt(&tx, &attempt).await?;
        super::run_attempt_persistence::save_attempt_steps(&tx, &steps).await?;
        let workflow_revision = super::runs::workflow_revision_for_run(&tx, &run).await?;
        let mut jobs = super::run_attempt_persistence::jobs_for_run(&tx, &run.id).await?;
        if let Some(stored) = jobs.iter_mut().find(|stored| stored.key == job.key) {
            *stored = job.clone();
        }
        reconcile_run(&mut run, &mut jobs, &workflow_revision, job.updated_at_unix)
            .map_err(PostgresError::from)?;
        super::run_attempt_persistence::save_run(&tx, &run).await?;
        tx.commit().await.map_err(PostgresError::internal)?;
        Ok(DispatchClaim {
            run,
            job,
            attempt,
            steps,
            workflow_revision,
        })
    }
}
