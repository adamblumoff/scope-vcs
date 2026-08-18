use super::{DispatchClaim, RunStore};
use crate::error::PostgresError;
use scope_domain::runs::run::StepConclusion;

impl RunStore {
    pub async fn start_attempt_step(
        &self,
        attempt_id: &str,
        token_hash: &str,
        step_index: u32,
        now_unix: u64,
    ) -> Result<DispatchClaim, PostgresError> {
        self.mutate_active_attempt(attempt_id, |run, job, attempt, steps| {
            attempt.start_step(run, job, steps, token_hash, step_index, now_unix)
        })
        .await
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn complete_attempt_step(
        &self,
        attempt_id: &str,
        token_hash: &str,
        step_index: u32,
        conclusion: StepConclusion,
        logs_truncated: bool,
        now_unix: u64,
    ) -> Result<DispatchClaim, PostgresError> {
        if matches!(conclusion, StepConclusion::Failed { .. }) {
            self.mutate_attempt(attempt_id, |_, job, attempt, steps| {
                attempt.complete_step(
                    job,
                    steps,
                    token_hash,
                    step_index,
                    conclusion,
                    logs_truncated,
                    now_unix,
                )
            })
            .await
        } else {
            self.mutate_active_attempt(attempt_id, |_, job, attempt, steps| {
                attempt.complete_step(
                    job,
                    steps,
                    token_hash,
                    step_index,
                    conclusion,
                    logs_truncated,
                    now_unix,
                )
            })
            .await
        }
    }
}
