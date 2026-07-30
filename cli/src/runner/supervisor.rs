use super::{RunnerConfig, attempt_control_client, unix_now};
use crate::api::attempt_heartbeat;
use scope_api_contract::ClaimRunResponse;
use std::{
    process::Command,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU8, AtomicU64, Ordering},
    },
    thread,
    time::Duration,
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub(super) enum AttemptStopReason {
    None = 0,
    Cancellation = 1,
    LeaseLost = 2,
    TimedOut = 3,
}

impl AttemptStopReason {
    fn from_u8(value: u8) -> Self {
        match value {
            1 => Self::Cancellation,
            2 => Self::LeaseLost,
            3 => Self::TimedOut,
            _ => Self::None,
        }
    }
}

pub(super) struct AttemptSupervisor {
    stop: Arc<AtomicBool>,
    reason: Arc<AtomicU8>,
    execution_deadline: Arc<AtomicU64>,
    execution_finished: Arc<AtomicBool>,
    handle: Option<thread::JoinHandle<()>>,
}

impl AttemptSupervisor {
    pub(super) fn start(config: RunnerConfig, claim: ClaimRunResponse) -> anyhow::Result<Self> {
        let client = attempt_control_client()?;
        let stop = Arc::new(AtomicBool::new(false));
        let reason = Arc::new(AtomicU8::new(AttemptStopReason::None as u8));
        let execution_deadline = Arc::new(AtomicU64::new(0));
        let execution_finished = Arc::new(AtomicBool::new(false));
        let thread_stop = Arc::clone(&stop);
        let thread_reason = Arc::clone(&reason);
        let thread_deadline = Arc::clone(&execution_deadline);
        let thread_execution_finished = Arc::clone(&execution_finished);
        let handle = thread::spawn(move || {
            let mut confirmed_lease_deadline = claim.lease_expires_at_unix;
            let mut next_heartbeat_at = unix_now();
            let mut pending_stop = AttemptStopReason::None;
            while !thread_stop.load(Ordering::Relaxed) {
                let now = unix_now();
                let execution_deadline = thread_deadline.load(Ordering::Relaxed);
                if pending_stop == AttemptStopReason::None
                    && !thread_execution_finished.load(Ordering::Relaxed)
                    && execution_deadline != 0
                    && now >= execution_deadline
                {
                    pending_stop = AttemptStopReason::TimedOut;
                }

                if pending_stop != AttemptStopReason::LeaseLost && now >= next_heartbeat_at {
                    match attempt_heartbeat(
                        &client,
                        &config.api_url,
                        &claim.attempt_token,
                        &claim.attempt_id,
                    ) {
                        Ok(status) => {
                            confirmed_lease_deadline = status.lease_expires_at_unix;
                            next_heartbeat_at = unix_now().saturating_add(15);
                            if status.cancellation_requested {
                                pending_stop = AttemptStopReason::Cancellation;
                            }
                        }
                        Err(error) => {
                            eprintln!("Attempt heartbeat failed: {error}");
                            next_heartbeat_at = unix_now().saturating_add(5);
                        }
                    }
                }

                let now = unix_now();
                if pending_stop == AttemptStopReason::None
                    && now.saturating_add(20) >= confirmed_lease_deadline
                {
                    pending_stop = AttemptStopReason::LeaseLost;
                }
                if matches!(
                    pending_stop,
                    AttemptStopReason::TimedOut | AttemptStopReason::Cancellation
                ) && thread_execution_finished.load(Ordering::Relaxed)
                {
                    pending_stop = AttemptStopReason::None;
                }
                if pending_stop != AttemptStopReason::None {
                    thread_reason.store(pending_stop as u8, Ordering::Relaxed);
                    return;
                }
                thread::sleep(Duration::from_secs(1));
            }
        });
        Ok(Self {
            stop,
            reason,
            execution_deadline,
            execution_finished,
            handle: Some(handle),
        })
    }

    pub(super) fn set_execution_deadline(&self, deadline_unix: u64) {
        self.execution_deadline
            .store(deadline_unix, Ordering::Relaxed);
    }

    pub(super) fn mark_execution_finished(&self) {
        self.execution_finished.store(true, Ordering::Relaxed);
        self.reason
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |reason| {
                match AttemptStopReason::from_u8(reason) {
                    AttemptStopReason::TimedOut | AttemptStopReason::Cancellation => {
                        Some(AttemptStopReason::None as u8)
                    }
                    _ => None,
                }
            })
            .ok();
    }

    pub(super) fn reason(&self) -> AttemptStopReason {
        let reason = AttemptStopReason::from_u8(self.reason.load(Ordering::Relaxed));
        if reason != AttemptStopReason::None || self.execution_finished.load(Ordering::Relaxed) {
            return reason;
        }
        let deadline = self.execution_deadline.load(Ordering::Relaxed);
        if deadline != 0 && unix_now() >= deadline {
            self.reason
                .store(AttemptStopReason::TimedOut as u8, Ordering::Relaxed);
            AttemptStopReason::TimedOut
        } else {
            AttemptStopReason::None
        }
    }

    pub(super) fn finish(&mut self) -> AttemptStopReason {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
        self.reason()
    }
}

impl Drop for AttemptSupervisor {
    fn drop(&mut self) {
        let _ = self.finish();
    }
}

pub(super) fn terminate_container(container_name: &str) -> bool {
    let removed = Command::new("docker")
        .args(["rm", "-f", container_name])
        .output();
    if removed.as_ref().is_ok_and(|output| output.status.success()) {
        return true;
    }
    let inspected = Command::new("docker")
        .args(["container", "inspect", container_name])
        .output();
    inspected.is_ok_and(|output| {
        !output.status.success()
            && String::from_utf8_lossy(&output.stderr)
                .to_ascii_lowercase()
                .contains("no such")
    })
}
