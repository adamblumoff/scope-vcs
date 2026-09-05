use crate::execution::CloudExecutionCoordinator;
use std::future::Future;
use tokio::task::JoinSet;

/// Each phase owns at most one batch. A slow provider operation cannot prevent
/// another phase from starting a new batch on the next control poll.
#[derive(Default)]
pub(super) struct CloudReconciliation {
    cleanup: PhaseTask,
    cancellation: PhaseTask,
    dispatch: PhaseTask,
}

impl CloudReconciliation {
    pub(super) fn poll(&mut self, execution: &CloudExecutionCoordinator) {
        let coordinator = execution.clone();
        self.cleanup.start_if_idle("cleanup", async move {
            coordinator.cleanup_terminal(crate::unix_now()?).await
        });
        let coordinator = execution.clone();
        self.cancellation.start_if_idle("cancellation", async move {
            coordinator.abort_canceled(crate::unix_now()?).await
        });
        let coordinator = execution.clone();
        self.dispatch.start_if_idle("dispatch", async move {
            coordinator.dispatch_available(crate::unix_now()?).await
        });
    }
}

/// Dropping the control loop aborts these tasks rather than detaching them.
/// Attempts and stop claims were persisted before provider I/O; their existing
/// leases and ambiguous-start reconciliation still own recovery after shutdown.
#[derive(Default)]
struct PhaseTask(JoinSet<()>);

impl PhaseTask {
    fn start_if_idle(
        &mut self,
        phase: &'static str,
        work: impl Future<Output = anyhow::Result<usize>> + Send + 'static,
    ) {
        while let Some(result) = self.0.try_join_next() {
            if let Err(error) = result {
                tracing::error!(phase, error = %error, "cloud reconciliation task panicked");
            }
        }
        if self.0.is_empty() {
            self.0.spawn(async move {
                match work.await {
                    Ok(processed) if processed > 0 => {
                        tracing::info!(phase, processed, "processed cloud reconciliation batch");
                    }
                    Ok(_) => {}
                    Err(error) => {
                        tracing::error!(phase, error = %error, "cloud reconciliation failed; will retry");
                    }
                }
            });
        }
    }
}

#[cfg(test)]
mod tests;
