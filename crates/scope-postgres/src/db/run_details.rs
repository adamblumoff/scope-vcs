use super::{RunStore, entities};
use crate::error::PostgresError;
use scope_domain::runs::{
    job::RunJob,
    run::{MAX_RUN_ATTEMPTS, Run, RunAttempt, RunAttemptStep},
    workflow::{MAX_WORKFLOW_JOBS, WorkflowRevision},
};
use sea_orm::{ColumnTrait, ConnectionTrait, EntityTrait, QueryFilter, QueryOrder, QuerySelect};
use std::collections::HashMap;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RunAttemptDetail {
    pub attempt: RunAttempt,
    pub steps: Vec<RunAttemptStep>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RunDetail {
    pub run: Run,
    pub jobs: Vec<RunJob>,
    pub workflow_revision: WorkflowRevision,
    pub attempts: Vec<RunAttemptDetail>,
}

impl RunStore {
    pub async fn run_detail(&self, run_id: &str) -> Result<Option<RunDetail>, PostgresError> {
        let tx = super::begin_metadata_read_snapshot(self.db.as_ref()).await?;
        let Some(run) = entities::run::Entity::find_by_id(run_id.to_string())
            .one(&tx)
            .await
            .map_err(PostgresError::internal)?
            .map(entities::run::Model::try_into_domain)
            .transpose()?
        else {
            tx.commit().await.map_err(PostgresError::internal)?;
            return Ok(None);
        };
        let workflow_revision = super::runs::workflow_revision_for_run(&tx, &run).await?;
        let jobs = super::run_attempt_persistence::jobs_for_run(&tx, run_id).await?;
        if jobs.is_empty() {
            return Err(PostgresError::internal_message(
                "run is missing its persisted jobs",
            ));
        }
        let attempts = run_attempt_details_with(&tx, run_id).await?;
        tx.commit().await.map_err(PostgresError::internal)?;
        Ok(Some(RunDetail {
            run,
            jobs,
            workflow_revision,
            attempts,
        }))
    }

    pub async fn run_attempt_details(
        &self,
        run_id: &str,
    ) -> Result<Vec<RunAttemptDetail>, PostgresError> {
        run_attempt_details_with(self.db.as_ref(), run_id).await
    }
}

async fn run_attempt_details_with<C>(
    conn: &C,
    run_id: &str,
) -> Result<Vec<RunAttemptDetail>, PostgresError>
where
    C: ConnectionTrait,
{
    let max_attempts = u64::from(MAX_RUN_ATTEMPTS)
        .checked_mul(u64::try_from(MAX_WORKFLOW_JOBS).map_err(|_| {
            PostgresError::internal_message("workflow job limit does not fit the database query")
        })?)
        .ok_or_else(|| PostgresError::internal_message("run attempt query limit overflow"))?;
    let attempts = entities::run_attempt::Entity::find()
        .filter(entities::run_attempt::Column::RunId.eq(run_id))
        .order_by_desc(entities::run_attempt::Column::CreatedAtUnix)
        .order_by_desc(entities::run_attempt::Column::Number)
        .limit(max_attempts)
        .all(conn)
        .await
        .map_err(PostgresError::internal)?;
    let attempt_ids = attempts
        .iter()
        .map(|attempt| attempt.id.clone())
        .collect::<Vec<_>>();
    let step_models = if attempt_ids.is_empty() {
        Vec::new()
    } else {
        entities::run_attempt_step::Entity::find()
            .filter(entities::run_attempt_step::Column::AttemptId.is_in(attempt_ids))
            .order_by_asc(entities::run_attempt_step::Column::AttemptId)
            .order_by_asc(entities::run_attempt_step::Column::StepIndex)
            .all(conn)
            .await
            .map_err(PostgresError::internal)?
    };
    let mut steps_by_attempt = HashMap::<String, Vec<RunAttemptStep>>::new();
    for step in step_models {
        let attempt_id = step.attempt_id.clone();
        steps_by_attempt
            .entry(attempt_id)
            .or_default()
            .push(step.try_into_domain()?);
    }
    let mut details = Vec::with_capacity(attempts.len());
    for attempt in attempts {
        let steps = steps_by_attempt.remove(&attempt.id).unwrap_or_default();
        let attempt = attempt.try_into_domain()?;
        attempt
            .validate_execution(&steps)
            .map_err(PostgresError::invalid_input)?;
        details.push(RunAttemptDetail { attempt, steps });
    }
    Ok(details)
}
