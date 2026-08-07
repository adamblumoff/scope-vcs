use super::{RunStore, StoredRunLog, entities};
use crate::error::PostgresError;
use scope_domain::runs::{
    job::RunJob,
    run::{MAX_RUN_ATTEMPTS, Run, RunAttempt, RunAttemptStep},
    workflow::WorkflowRevision,
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

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StoredAttemptStepLogs {
    pub logs: Vec<StoredRunLog>,
    pub logs_truncated: bool,
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

    pub async fn attempt_step_logs_after(
        &self,
        run_id: &str,
        attempt_id: &str,
        step_index: u32,
        after: u64,
        limit: u64,
    ) -> Result<StoredAttemptStepLogs, PostgresError> {
        let attempt = entities::run_attempt::Entity::find_by_id(attempt_id.to_string())
            .filter(entities::run_attempt::Column::RunId.eq(run_id))
            .one(self.db.as_ref())
            .await
            .map_err(PostgresError::internal)?
            .ok_or_else(|| PostgresError::not_found("run attempt not found"))?;
        let step_index = i32::try_from(step_index)
            .map_err(|_| PostgresError::invalid_input("step index is too large"))?;
        let step_exists =
            entities::run_attempt_step::Entity::find_by_id((attempt_id.to_string(), step_index))
                .one(self.db.as_ref())
                .await
                .map_err(PostgresError::internal)?
                .is_some();
        if !step_exists {
            return Err(PostgresError::not_found("run attempt step not found"));
        }
        let after = i64::try_from(after)
            .map_err(|_| PostgresError::invalid_input("run log cursor is too large"))?;
        let logs = entities::run_log::Entity::find()
            .filter(entities::run_log::Column::RunId.eq(run_id))
            .filter(entities::run_log::Column::AttemptId.eq(attempt_id))
            .filter(entities::run_log::Column::StepIndex.eq(step_index))
            .filter(entities::run_log::Column::Position.gt(after))
            .order_by_asc(entities::run_log::Column::Position)
            .limit(limit)
            .all(self.db.as_ref())
            .await
            .map_err(PostgresError::internal)?
            .into_iter()
            .map(|model| -> Result<StoredRunLog, PostgresError> {
                let position = entities::i64_to_u64(model.position, "run log position")?;
                let run_id = model.run_id.clone();
                Ok(StoredRunLog {
                    position,
                    run_id,
                    chunk: model.try_into_domain()?,
                })
            })
            .collect::<Result<Vec<_>, _>>()?;
        Ok(StoredAttemptStepLogs {
            logs,
            logs_truncated: attempt.logs_truncated,
        })
    }

    pub async fn run_logs_after(
        &self,
        run_id: &str,
        after: u64,
        limit: u64,
    ) -> Result<Vec<StoredRunLog>, PostgresError> {
        let after = i64::try_from(after)
            .map_err(|_| PostgresError::invalid_input("run log cursor is too large"))?;
        entities::run_log::Entity::find()
            .filter(entities::run_log::Column::RunId.eq(run_id))
            .filter(entities::run_log::Column::Position.gt(after))
            .order_by_asc(entities::run_log::Column::Position)
            .limit(limit)
            .all(self.db.as_ref())
            .await
            .map_err(PostgresError::internal)?
            .into_iter()
            .map(|model| {
                let position = entities::i64_to_u64(model.position, "run log position")?;
                let run_id = model.run_id.clone();
                Ok(StoredRunLog {
                    position,
                    run_id,
                    chunk: model.try_into_domain()?,
                })
            })
            .collect()
    }

    pub async fn next_attempt_log_sequence(&self, attempt_id: &str) -> Result<u64, PostgresError> {
        let last = entities::run_log::Entity::find()
            .filter(entities::run_log::Column::AttemptId.eq(attempt_id))
            .order_by_desc(entities::run_log::Column::Sequence)
            .one(self.db.as_ref())
            .await
            .map_err(PostgresError::internal)?;
        match last {
            Some(log) => entities::i64_to_u64(log.sequence, "run log sequence")?
                .checked_add(1)
                .ok_or_else(|| PostgresError::conflict("run log sequence overflow")),
            None => Ok(1),
        }
    }
}

async fn run_attempt_details_with<C>(
    conn: &C,
    run_id: &str,
) -> Result<Vec<RunAttemptDetail>, PostgresError>
where
    C: ConnectionTrait,
{
    let attempts = entities::run_attempt::Entity::find()
        .filter(entities::run_attempt::Column::RunId.eq(run_id))
        .order_by_desc(entities::run_attempt::Column::CreatedAtUnix)
        .order_by_desc(entities::run_attempt::Column::Number)
        .limit(u64::from(MAX_RUN_ATTEMPTS))
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
