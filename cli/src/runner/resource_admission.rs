use super::resources::{ResourceCapacity, ResourceLimits, ResourceUsage, scope_container_usage};
use anyhow::bail;
use scope_domain::runs::runner::RunnerMaxConcurrentJobs;
use std::sync::{Arc, Mutex};

type CapacityDetector = dyn Fn() -> anyhow::Result<ResourceCapacity> + Send + Sync;
type UsageDetector = dyn Fn(&str) -> anyhow::Result<ResourceUsage> + Send + Sync;

#[derive(Clone)]
pub(super) struct ResourceAdmissionCoordinator {
    inner: Arc<CoordinatorInner>,
}

struct CoordinatorInner {
    max_concurrent_jobs: RunnerMaxConcurrentJobs,
    limits: ResourceLimits,
    detector: Box<CapacityDetector>,
    usage_detector: Box<UsageDetector>,
    state: Mutex<AdmissionState>,
}

#[derive(Default)]
struct AdmissionState {
    active: Vec<ActiveReservation>,
    pending: u8,
    next_reservation_id: u64,
}

struct ActiveReservation {
    id: u64,
    container_name: String,
}

#[derive(Clone, Copy)]
enum ReservationState {
    Pending,
    Active,
}

pub(super) struct ResourceAdmissionReservation {
    inner: Arc<CoordinatorInner>,
    id: u64,
    state: ReservationState,
}

impl ResourceAdmissionCoordinator {
    pub(super) fn new(
        max_concurrent_jobs: RunnerMaxConcurrentJobs,
        limits: ResourceLimits,
    ) -> Self {
        Self::with_detectors(
            max_concurrent_jobs,
            limits,
            ResourceCapacity::detect,
            scope_container_usage,
        )
    }

    fn with_detectors<D, U>(
        max_concurrent_jobs: RunnerMaxConcurrentJobs,
        limits: ResourceLimits,
        detector: D,
        usage_detector: U,
    ) -> Self
    where
        D: Fn() -> anyhow::Result<ResourceCapacity> + Send + Sync + 'static,
        U: Fn(&str) -> anyhow::Result<ResourceUsage> + Send + Sync + 'static,
    {
        Self {
            inner: Arc::new(CoordinatorInner {
                max_concurrent_jobs,
                limits,
                detector: Box::new(detector),
                usage_detector: Box::new(usage_detector),
                state: Mutex::new(AdmissionState::default()),
            }),
        }
    }

    #[cfg(test)]
    pub(super) fn for_test<D, U>(
        max_concurrent_jobs: RunnerMaxConcurrentJobs,
        limits: ResourceLimits,
        detector: D,
        usage_detector: U,
    ) -> Self
    where
        D: Fn() -> anyhow::Result<ResourceCapacity> + Send + Sync + 'static,
        U: Fn(&str) -> anyhow::Result<ResourceUsage> + Send + Sync + 'static,
    {
        Self::with_detectors(max_concurrent_jobs, limits, detector, usage_detector)
    }

    pub(super) fn reserve(&self) -> anyhow::Result<ResourceAdmissionReservation> {
        let mut state = self
            .inner
            .state
            .lock()
            .map_err(|_| anyhow::anyhow!("runner resource admission state is poisoned"))?;
        debug_assert!(
            state.active.len() + usize::from(state.pending)
                <= usize::from(self.inner.max_concurrent_jobs.get()),
            "runner resource admission count exceeds configured slots"
        );
        if state.active.len() + usize::from(state.pending)
            >= usize::from(self.inner.max_concurrent_jobs.get())
        {
            bail!("all runner resource slots are reserved");
        }
        let active_usage = state
            .active
            .iter()
            .map(|reservation| (self.inner.usage_detector)(&reservation.container_name))
            .collect::<anyhow::Result<Vec<_>>>()?;
        (self.inner.detector)()?.ensure_admission(
            &self.inner.limits,
            &active_usage,
            state.pending,
        )?;
        let id = state.next_reservation_id;
        state.next_reservation_id = state
            .next_reservation_id
            .checked_add(1)
            .ok_or_else(|| anyhow::anyhow!("runner resource reservation identity overflow"))?;
        state.pending += 1;
        Ok(ResourceAdmissionReservation {
            inner: Arc::clone(&self.inner),
            id,
            state: ReservationState::Pending,
        })
    }
}

impl ResourceAdmissionReservation {
    pub(super) fn activate(&mut self, container_name: String) {
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
        state.active.push(ActiveReservation {
            id: self.id,
            container_name,
        });
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
                let index = state
                    .active
                    .iter()
                    .position(|reservation| reservation.id == self.id)
                    .expect("active runner resource reservation must be registered");
                state.active.swap_remove(index);
            }
        }
    }
}
