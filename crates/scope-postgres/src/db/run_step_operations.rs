use super::{DispatchClaim, RunStore, entities};
use crate::error::PostgresError;
use scope_domain::runs::run::StepConclusion;
use sea_orm::{ColumnTrait, EntityTrait, PaginatorTrait, QueryFilter};

impl RunStore {
    pub async fn start_attempt_step(
        &self,
        attempt_id: &str,
        runner_id: &str,
        token_hash: &str,
        step_index: u32,
        now_unix: u64,
    ) -> Result<DispatchClaim, PostgresError> {
        self.mutate_active_attempt(attempt_id, |run, job, attempt, steps| {
            attempt.start_step(run, job, steps, runner_id, token_hash, step_index, now_unix)
        })
        .await
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn complete_attempt_step(
        &self,
        attempt_id: &str,
        runner_id: &str,
        token_hash: &str,
        step_index: u32,
        conclusion: StepConclusion,
        now_unix: u64,
    ) -> Result<DispatchClaim, PostgresError> {
        let step_count = entities::run_attempt_step::Entity::find()
            .filter(entities::run_attempt_step::Column::AttemptId.eq(attempt_id))
            .count(self.db.as_ref())
            .await
            .map_err(PostgresError::internal)?;
        let last_step = usize::try_from(step_index)
            .ok()
            .and_then(|index| u64::try_from(index + 1).ok())
            == Some(step_count);
        if matches!(conclusion, StepConclusion::Failed { .. }) || last_step {
            self.mutate_attempt(attempt_id, |_, job, attempt, steps| {
                attempt.complete_step(
                    job, steps, runner_id, token_hash, step_index, conclusion, now_unix,
                )
            })
            .await
        } else {
            self.mutate_active_attempt(attempt_id, |_, job, attempt, steps| {
                attempt.complete_step(
                    job, steps, runner_id, token_hash, step_index, conclusion, now_unix,
                )
            })
            .await
        }
    }
}
