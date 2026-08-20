use crate::settings::WorkerRole;
use axum::{Router, extract::State, http::StatusCode, routing::get};
use std::{
    net::{Ipv6Addr, SocketAddr},
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    time::Duration,
};

#[derive(Clone)]
pub(crate) struct WorkerHealth {
    state: Arc<WorkerHealthState>,
}

struct WorkerHealthState {
    schema_ready: AtomicBool,
    last_successful_poll_unix: [AtomicU64; 3],
    required_roles: [bool; 3],
    stale_after_secs: u64,
}

impl WorkerHealth {
    pub(crate) fn new(poll_interval: Duration, role: WorkerRole) -> Self {
        let stale_after_secs = poll_interval.as_secs().saturating_mul(3).max(10);
        Self {
            state: Arc::new(WorkerHealthState {
                schema_ready: AtomicBool::new(false),
                last_successful_poll_unix: std::array::from_fn(|_| AtomicU64::new(0)),
                required_roles: [
                    role.runs_control(),
                    role.runs_compaction(),
                    role.runs_cleanup(),
                ],
                stale_after_secs,
            }),
        }
    }

    pub(crate) fn mark_schema_waiting(&self) {
        self.state.schema_ready.store(false, Ordering::Release);
    }

    pub(crate) fn mark_poll_succeeded(&self, role: WorkerRole, now_unix: u64) {
        let index = concrete_role_index(role);
        self.state.last_successful_poll_unix[index].store(now_unix, Ordering::Release);
        self.state.schema_ready.store(true, Ordering::Release);
    }

    pub(crate) async fn serve(self, port: u16) -> anyhow::Result<()> {
        let addr = SocketAddr::from((Ipv6Addr::UNSPECIFIED, port));
        let app = Router::new()
            .route("/healthz", get(healthz))
            .with_state(self);
        let listener = tokio::net::TcpListener::bind(addr).await?;
        tracing::info!(%addr, "starting worker health server");
        axum::serve(listener, app)
            .with_graceful_shutdown(super::shutdown_signal())
            .await?;
        Ok(())
    }

    fn is_ready_at(&self, now_unix: u64) -> bool {
        if !self.state.schema_ready.load(Ordering::Acquire) {
            return false;
        }
        self.state
            .required_roles
            .iter()
            .zip(&self.state.last_successful_poll_unix)
            .all(|(required, last_success)| {
                if !required {
                    return true;
                }
                let last_success = last_success.load(Ordering::Acquire);
                last_success > 0
                    && now_unix.saturating_sub(last_success) <= self.state.stale_after_secs
            })
    }
}

fn concrete_role_index(role: WorkerRole) -> usize {
    match role {
        WorkerRole::Control => 0,
        WorkerRole::Compaction => 1,
        WorkerRole::Cleanup => 2,
        WorkerRole::All => panic!("health updates require one concrete worker role"),
    }
}

async fn healthz(State(health): State<WorkerHealth>) -> StatusCode {
    match super::unix_now() {
        Ok(now_unix) if health.is_ready_at(now_unix) => StatusCode::OK,
        _ => StatusCode::SERVICE_UNAVAILABLE,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn health_requires_matching_schema_and_a_recent_poll() {
        let health = WorkerHealth::new(Duration::from_secs(1), WorkerRole::Control);
        assert!(!health.is_ready_at(100));

        health.mark_poll_succeeded(WorkerRole::Control, 100);
        assert!(health.is_ready_at(110));
        assert!(!health.is_ready_at(111));

        health.mark_schema_waiting();
        assert!(!health.is_ready_at(100));
    }

    #[test]
    fn all_role_health_requires_every_loop() {
        let health = WorkerHealth::new(Duration::from_secs(1), WorkerRole::All);
        health.mark_poll_succeeded(WorkerRole::Control, 100);
        health.mark_poll_succeeded(WorkerRole::Compaction, 100);
        assert!(!health.is_ready_at(100));

        health.mark_poll_succeeded(WorkerRole::Cleanup, 100);
        assert!(health.is_ready_at(100));
    }
}
