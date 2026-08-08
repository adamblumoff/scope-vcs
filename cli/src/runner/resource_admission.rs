use super::resources::{ResourceCapacity, ResourceLimits};
use anyhow::bail;
use scope_domain::runs::runner::RunnerMaxConcurrentJobs;
use std::sync::{Arc, Mutex};

type CapacityDetector = dyn Fn() -> anyhow::Result<ResourceCapacity> + Send + Sync;

#[derive(Clone)]
pub(super) struct ResourceAdmissionCoordinator {
    inner: Arc<CoordinatorInner>,
}

struct CoordinatorInner {
    max_concurrent_jobs: RunnerMaxConcurrentJobs,
    limits: ResourceLimits,
    detector: Box<CapacityDetector>,
    state: Mutex<AdmissionState>,
}

#[derive(Default)]
struct AdmissionState {
    active: u8,
    pending: u8,
}

#[derive(Clone, Copy)]
enum ReservationState {
    Pending,
    Active,
}

pub(super) struct ResourceAdmissionReservation {
    inner: Arc<CoordinatorInner>,
    state: ReservationState,
}

impl ResourceAdmissionCoordinator {
    pub(super) fn new(
        max_concurrent_jobs: RunnerMaxConcurrentJobs,
        limits: ResourceLimits,
    ) -> Self {
        Self::with_detector(max_concurrent_jobs, limits, ResourceCapacity::detect)
    }

    fn with_detector<D>(
        max_concurrent_jobs: RunnerMaxConcurrentJobs,
        limits: ResourceLimits,
        detector: D,
    ) -> Self
    where
        D: Fn() -> anyhow::Result<ResourceCapacity> + Send + Sync + 'static,
    {
        Self {
            inner: Arc::new(CoordinatorInner {
                max_concurrent_jobs,
                limits,
                detector: Box::new(detector),
                state: Mutex::new(AdmissionState::default()),
            }),
        }
    }

    #[cfg(test)]
    pub(super) fn for_test<D>(
        max_concurrent_jobs: RunnerMaxConcurrentJobs,
        limits: ResourceLimits,
        detector: D,
    ) -> Self
    where
        D: Fn() -> anyhow::Result<ResourceCapacity> + Send + Sync + 'static,
    {
        Self::with_detector(max_concurrent_jobs, limits, detector)
    }

    pub(super) fn reserve(&self) -> anyhow::Result<ResourceAdmissionReservation> {
        let mut state = self
            .inner
            .state
            .lock()
            .map_err(|_| anyhow::anyhow!("runner resource admission state is poisoned"))?;
        debug_assert!(
            state.active + state.pending <= self.inner.max_concurrent_jobs.get(),
            "runner resource admission count exceeds configured slots"
        );
        if state.active + state.pending >= self.inner.max_concurrent_jobs.get() {
            bail!("all runner resource slots are reserved");
        }
        (self.inner.detector)()?.ensure_admission(
            &self.inner.limits,
            state.active,
            state.pending,
        )?;
        state.pending += 1;
        Ok(ResourceAdmissionReservation {
            inner: Arc::clone(&self.inner),
            state: ReservationState::Pending,
        })
    }
}

impl ResourceAdmissionReservation {
    pub(super) fn activate(&mut self) {
        if matches!(self.state, ReservationState::Active) {
            return;
        }
        let mut state = self
            .inner
            .state
            .lock()
            .expect("runner resource admission state must not be poisoned");
        assert!(
            state.pending > 0,
            "runner resource admission pending count is inconsistent"
        );
        state.pending -= 1;
        state.active += 1;
        self.state = ReservationState::Active;
    }

    pub(super) fn limits(&self) -> &ResourceLimits {
        &self.inner.limits
    }
}

impl Drop for ResourceAdmissionReservation {
    fn drop(&mut self) {
        let Ok(mut state) = self.inner.state.lock() else {
            return;
        };
        match self.state {
            ReservationState::Pending => {
                assert!(state.pending > 0);
                state.pending -= 1;
            }
            ReservationState::Active => {
                assert!(state.active > 0);
                state.active -= 1;
            }
        }
    }
}
