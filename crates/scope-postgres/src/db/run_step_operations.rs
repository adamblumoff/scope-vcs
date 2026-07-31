use super::{DispatchClaim, RunStore};
use crate::error::PostgresError;
use scope_domain::runs::run::StepConclusion;

impl RunStore {
    pub async fn start_attempt_step(
        &self,
        attempt_id: &str,
        runner_id: &str,
        token_hash: &str,
        step_index: u32,
        now_unix: u64,
    ) -> Result<DispatchClaim, PostgresError> {
        self.mutate_attempt(attempt_id, |run, attempt, steps| {
            attempt.start_step(run, steps, runner_id, token_hash, step_index, now_unix)
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
        self.mutate_attempt(attempt_id, |run, attempt, steps| {
            attempt.complete_step(
                run, steps, runner_id, token_hash, step_index, conclusion, now_unix,
            )
        })
        .await
    }
}
