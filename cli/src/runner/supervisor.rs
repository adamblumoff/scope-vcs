use super::{
    RunnerConfig, attempt_control_client, cache,
    container::{job_container_name, stop_container},
    resources::{storage_has_emergency_capacity_at, transient_storage_root},
    unix_now,
};
use crate::api::attempt_heartbeat;
use anyhow::{Context, bail};
use scope_api_contract::ClaimRunResponse;
use std::{
    path::Path,
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
    storage_pressure: Arc<AtomicBool>,
    control_handle: Option<thread::JoinHandle<()>>,
    storage_handle: Option<thread::JoinHandle<()>>,
}

impl AttemptSupervisor {
    pub(super) fn start(config: RunnerConfig, claim: ClaimRunResponse) -> anyhow::Result<Self> {
        let client = attempt_control_client()?;
        let stop = Arc::new(AtomicBool::new(false));
        let reason = Arc::new(AtomicU8::new(AttemptStopReason::None as u8));
        let execution_deadline = Arc::new(AtomicU64::new(0));
        let execution_finished = Arc::new(AtomicBool::new(false));
        let storage_pressure = Arc::new(AtomicBool::new(false));
        let thread_stop = Arc::clone(&stop);
        let thread_reason = Arc::clone(&reason);
        let thread_deadline = Arc::clone(&execution_deadline);
        let thread_execution_finished = Arc::clone(&execution_finished);
        let control_config = config.clone();
        let control_claim = claim.clone();
        let control_handle = thread::spawn(move || {
            let mut confirmed_lease_deadline = control_claim.lease_expires_at_unix;
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
                        &control_config.api_url,
                        &control_claim.attempt_token,
                        &control_claim.attempt_id,
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
            storage_pressure,
            control_handle: Some(control_handle),
            storage_handle: None,
        })
    }

    pub(super) fn start_storage_monitor(
        &mut self,
        config: RunnerConfig,
        attempt_id: &str,
        work_dir: &Path,
    ) -> anyhow::Result<()> {
        if self.storage_handle.is_some() {
            bail!("attempt storage monitor is already running");
        }
        let transient_storage_root = transient_storage_root()?;
        let work_dir = work_dir
            .canonicalize()
            .context("resolve runner work directory for storage monitoring")?;
        if !storage_is_safe(&config, &transient_storage_root, &work_dir)? {
            self.storage_pressure.store(true, Ordering::Relaxed);
            bail!("runner storage crossed its emergency floor before source preparation");
        }
        let storage_stop = Arc::clone(&self.stop);
        let thread_storage_pressure = Arc::clone(&self.storage_pressure);
        let storage_attempt_id = attempt_id.to_string();
        let container_name = job_container_name(&storage_attempt_id);
        self.storage_handle = Some(thread::spawn(move || {
            run_storage_monitor(
                &storage_stop,
                &thread_storage_pressure,
                &storage_attempt_id,
                || storage_is_safe(&config, &transient_storage_root, &work_dir),
                || stop_container(&container_name),
                Duration::from_secs(1),
            );
        }));
        Ok(())
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

    pub(super) fn storage_pressure_triggered(&self) -> bool {
        self.storage_pressure.load(Ordering::Relaxed)
    }

    pub(super) fn finish(&mut self) -> AttemptStopReason {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(handle) = self.control_handle.take() {
            let _ = handle.join();
        }
        if let Some(handle) = self.storage_handle.take() {
            let _ = handle.join();
        }
        self.reason()
    }
}

fn storage_is_safe(
    config: &RunnerConfig,
    docker_root: &Path,
    work_dir: &Path,
) -> anyhow::Result<bool> {
    Ok(cache::has_emergency_capacity(config)?
        && storage_has_emergency_capacity_at(docker_root)?
        && storage_has_emergency_capacity_at(work_dir)?)
}

fn run_storage_monitor(
    stop: &AtomicBool,
    storage_pressure: &AtomicBool,
    attempt_id: &str,
    mut storage_is_safe: impl FnMut() -> anyhow::Result<bool>,
    mut stop_execution: impl FnMut() -> anyhow::Result<()>,
    interval: Duration,
) {
    let mut pressure_latched = false;
    while !stop.load(Ordering::Relaxed) {
        if !pressure_latched {
            pressure_latched = match storage_is_safe() {
                Ok(true) => false,
                Ok(false) => {
                    eprintln!(
                        "Runner storage crossed its emergency floor; stopping attempt {attempt_id}"
                    );
                    true
                }
                Err(error) => {
                    eprintln!(
                        "Could not inspect runner storage; stopping attempt {attempt_id} to protect the host: {error:#}"
                    );
                    true
                }
            };
        }
        if pressure_latched {
            storage_pressure.store(true, Ordering::Relaxed);
            if let Err(error) = stop_execution() {
                eprintln!("Could not stop attempt {attempt_id} after storage pressure: {error:#}");
            }
        }
        thread::sleep(interval);
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;

    #[test]
    fn storage_monitor_fails_closed_and_stops_execution() {
        let stop = AtomicBool::new(false);
        let storage_pressure = AtomicBool::new(false);
        let stop_called = Cell::new(false);

        run_storage_monitor(
            &stop,
            &storage_pressure,
            "attempt-1",
            || Err(anyhow::anyhow!("capacity unavailable")),
            || {
                stop_called.set(true);
                stop.store(true, Ordering::Relaxed);
                Ok(())
            },
            Duration::ZERO,
        );

        assert!(storage_pressure.load(Ordering::Relaxed));
        assert!(stop_called.get());
    }

    #[test]
    fn storage_monitor_retries_a_failed_stop() {
        let stop = AtomicBool::new(false);
        let storage_pressure = AtomicBool::new(false);
        let stop_attempts = Cell::new(0_u8);

        run_storage_monitor(
            &stop,
            &storage_pressure,
            "attempt-1",
            || Ok(false),
            || {
                let attempt = stop_attempts.get().saturating_add(1);
                stop_attempts.set(attempt);
                if attempt == 1 {
                    Err(anyhow::anyhow!("Docker was busy"))
                } else {
                    stop.store(true, Ordering::Relaxed);
                    Ok(())
                }
            },
            Duration::ZERO,
        );

        assert_eq!(stop_attempts.get(), 2);
        assert!(storage_pressure.load(Ordering::Relaxed));
    }

    #[test]
    fn storage_monitor_keeps_enforcing_after_a_successful_stop() {
        let stop = AtomicBool::new(false);
        let storage_pressure = AtomicBool::new(false);
        let stop_attempts = Cell::new(0_u8);

        run_storage_monitor(
            &stop,
            &storage_pressure,
            "attempt-1",
            || Ok(false),
            || {
                let attempt = stop_attempts.get().saturating_add(1);
                stop_attempts.set(attempt);
                if attempt == 2 {
                    stop.store(true, Ordering::Relaxed);
                }
                Ok(())
            },
            Duration::ZERO,
        );

        assert_eq!(stop_attempts.get(), 2);
        assert!(storage_pressure.load(Ordering::Relaxed));
    }
}
