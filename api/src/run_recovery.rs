use crate::{persistence::unix_now, state::AppState};
use scope_api_contract::RunChangeKind;
use scope_postgres::error::PostgresErrorKind;
use std::time::Duration;

const RECOVERY_BATCH_SIZE: u64 = 100;
const RECOVERY_INTERVAL: Duration = Duration::from_secs(15);

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct AttemptRecoveryReport {
    pub(crate) requeued: usize,
    pub(crate) lost: usize,
}

pub(crate) async fn reconcile_expired_attempts(
    state: &AppState,
    now_unix: u64,
) -> Result<AttemptRecoveryReport, scope_postgres::error::PostgresError> {
    let attempt_ids = state
        .metadata
        .runs()
        .expired_attempt_ids(now_unix, RECOVERY_BATCH_SIZE)
        .await?;
    let mut report = AttemptRecoveryReport::default();
    for attempt_id in attempt_ids {
        match state
            .metadata
            .runs()
            .expire_attempt(&attempt_id, now_unix)
            .await
        {
            Ok(claim) => {
                state
                    .publish_run_change(
                        claim.run.workflow.repository_id(),
                        claim.run.id.clone(),
                        RunChangeKind::StatusChanged,
                    )
                    .await;
                if claim.run.state == scope_domain::runs::run::RunState::Queued {
                    report.requeued += 1;
                } else {
                    report.lost += 1;
                }
            }
            Err(error)
                if matches!(
                    error.kind,
                    PostgresErrorKind::Conflict | PostgresErrorKind::NotFound
                ) =>
            {
                tracing::debug!(
                    attempt_id,
                    "run attempt changed while lease recovery was reconciling it"
                );
            }
            Err(error) => return Err(error),
        }
    }
    Ok(report)
}

impl AppState {
    pub(crate) fn start_run_attempt_recovery(&self) {
        let state = self.clone();
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(RECOVERY_INTERVAL);
            loop {
                interval.tick().await;
                let now_unix = match unix_now() {
                    Ok(now) => now,
                    Err(error) => {
                        tracing::warn!(?error, "failed to read time for run attempt recovery");
                        continue;
                    }
                };
                match reconcile_expired_attempts(&state, now_unix).await {
                    Ok(report) if report != AttemptRecoveryReport::default() => {
                        tracing::info!(
                            requeued = report.requeued,
                            lost = report.lost,
                            "reconciled expired run attempt leases"
                        );
                    }
                    Ok(_) => {}
                    Err(error) => {
                        tracing::warn!(
                            error = %error.message,
                            "failed to reconcile expired run attempt leases"
                        );
                    }
                }
            }
        });
    }
}
