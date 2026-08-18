use super::{
    DispatchClaim, RunStore, entities,
    run_attempt_persistence::{jobs_for_run, locked_job, locked_run, save_job, save_run},
    runs::{DispatchOffer, unique_conflict, workflow_revision_for_run},
};
use crate::error::PostgresError;
use scope_domain::runs::{
    job::reconcile_run,
    run::{ExecutionProvider, RunJobState},
};
use sea_orm::{
    ConnectionTrait, DatabaseBackend, EntityTrait, IntoActiveModel, Statement, TransactionTrait,
};

#[derive(Clone, Debug)]
pub struct CloudAttemptAbort {
    pub attempt_id: String,
    pub external_run_id: String,
}

impl RunStore {
    pub async fn claim_cloud_attempt_aborts(
        &self,
        now_unix: u64,
        limit: u64,
    ) -> Result<Vec<CloudAttemptAbort>, PostgresError> {
        let rows = self
            .db
            .query_all(Statement::from_sql_and_values(
                DatabaseBackend::Postgres,
                "WITH candidates AS (
               SELECT attempt.id FROM scope_run_attempts attempt
               JOIN scope_runs run ON run.id = attempt.run_id
               WHERE run.cancellation_requested = TRUE
                 AND attempt.state IN ('dispatching', 'running')
                 AND attempt.external_run_id IS NOT NULL
                 AND attempt.provider_abort_requested_at_unix IS NULL
               ORDER BY attempt.created_at_unix, attempt.id
               FOR UPDATE OF attempt SKIP LOCKED LIMIT $1
             )
             UPDATE scope_run_attempts attempt
             SET provider_abort_requested_at_unix = $2
             FROM candidates WHERE attempt.id = candidates.id
             RETURNING attempt.id, attempt.external_run_id",
                [
                    i64::try_from(limit)
                        .map_err(PostgresError::internal)?
                        .into(),
                    i64::try_from(now_unix)
                        .map_err(PostgresError::internal)?
                        .into(),
                ],
            ))
            .await
            .map_err(PostgresError::internal)?;
        rows.into_iter()
            .map(|row| {
                Ok(CloudAttemptAbort {
                    attempt_id: row.try_get("", "id").map_err(PostgresError::internal)?,
                    external_run_id: row
                        .try_get("", "external_run_id")
                        .map_err(PostgresError::internal)?,
                })
            })
            .collect()
    }

    pub async fn release_cloud_attempt_abort(&self, attempt_id: &str) -> Result<(), PostgresError> {
        self.db.execute(Statement::from_sql_and_values(DatabaseBackend::Postgres,
            "UPDATE scope_run_attempts SET provider_abort_requested_at_unix = NULL WHERE id = $1 AND state IN ('dispatching', 'running')",
            [attempt_id.into()])).await.map_err(PostgresError::internal)?;
        Ok(())
    }

    pub async fn active_cloud_attempt_count(&self) -> Result<u64, PostgresError> {
        let row = self.db.query_one(Statement::from_string(
            DatabaseBackend::Postgres,
            "SELECT COUNT(*)::bigint AS count FROM scope_run_attempts WHERE state IN ('dispatching', 'running')",
        )).await.map_err(PostgresError::internal)?
            .ok_or_else(|| PostgresError::internal_message("active attempt count is missing"))?;
        let count = row
            .try_get::<i64>("", "count")
            .map_err(PostgresError::internal)?;
        u64::try_from(count).map_err(PostgresError::internal)
    }

    pub async fn next_dispatchable_job(&self) -> Result<Option<DispatchOffer>, PostgresError> {
        let Some(row) = self
            .db
            .query_one(Statement::from_string(
                DatabaseBackend::Postgres,
                "SELECT job.run_id, job.job_key
                 FROM scope_run_jobs job
                 JOIN scope_runs run ON run.id = job.run_id
                 WHERE job.state = 'queued'
                   AND run.state IN ('queued', 'dispatching', 'running')
                   AND run.cancellation_requested = FALSE
                 ORDER BY job.created_at_unix, job.run_id, job.job_key
                 LIMIT 1",
            ))
            .await
            .map_err(PostgresError::internal)?
        else {
            return Ok(None);
        };
        let run_id = row
            .try_get::<String>("", "run_id")
            .map_err(PostgresError::internal)?;
        let job_key = row
            .try_get::<String>("", "job_key")
            .map_err(PostgresError::internal)?;
        let job = entities::run_job::Entity::find_by_id((run_id.clone(), job_key))
            .one(self.db.as_ref())
            .await
            .map_err(PostgresError::internal)?
            .ok_or_else(|| PostgresError::not_found("run job not found"))?
            .try_into_domain()?;
        let run = entities::run::Entity::find_by_id(run_id)
            .one(self.db.as_ref())
            .await
            .map_err(PostgresError::internal)?
            .ok_or_else(|| PostgresError::not_found("run not found"))?
            .try_into_domain()?;
        Ok(
            (job.state == RunJobState::Queued && !run.cancellation_requested)
                .then_some(DispatchOffer { run, job }),
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn dispatch_job(
        &self,
        run_id: &str,
        job_key: &str,
        attempt_id: &str,
        token_hash: &str,
        runtime_version: &str,
        now_unix: u64,
        lease_expires_at_unix: u64,
    ) -> Result<DispatchClaim, PostgresError> {
        let tx = self.db.begin().await.map_err(PostgresError::internal)?;
        let run_snapshot = entities::run::Entity::find_by_id(run_id.to_string())
            .one(&tx)
            .await
            .map_err(PostgresError::internal)?
            .ok_or_else(|| PostgresError::not_found("run not found"))?
            .try_into_domain()?;
        let mut job = locked_job(&tx, run_id, job_key).await?;
        let workflow_revision = workflow_revision_for_run(&tx, &run_snapshot).await?;
        let definition = workflow_revision
            .definition()
            .job(&job.key)
            .ok_or_else(|| PostgresError::internal_message("run job definition is missing"))?;
        let (attempt, steps) = job
            .dispatch(
                &run_snapshot,
                definition,
                attempt_id,
                token_hash,
                ExecutionProvider::Northflank,
                runtime_version,
                now_unix,
                lease_expires_at_unix,
            )
            .map_err(PostgresError::from)?;

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
        tx.commit().await.map_err(PostgresError::internal)?;
        Ok(DispatchClaim {
            run,
            job,
            attempt,
            steps,
            workflow_revision,
        })
    }

    pub async fn record_external_run_id(
        &self,
        attempt_id: &str,
        external_run_id: &str,
    ) -> Result<(), PostgresError> {
        if external_run_id.trim().is_empty() {
            return Err(PostgresError::invalid_input("external run id is required"));
        }
        let result = self
            .db
            .execute(Statement::from_sql_and_values(
                DatabaseBackend::Postgres,
                "UPDATE scope_run_attempts
             SET external_run_id = $2
             WHERE id = $1
               AND (external_run_id IS NULL OR external_run_id = $2)",
                [attempt_id.into(), external_run_id.into()],
            ))
            .await
            .map_err(PostgresError::internal)?;
        if result.rows_affected() != 1 {
            return Err(PostgresError::conflict(
                "attempt cannot accept the external run id",
            ));
        }
        Ok(())
    }
}
