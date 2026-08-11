use super::resources::{
    ReservedResourceUsage, ResourceCapacity, ResourceLimits, ResourceUsage, scope_container_usage,
};
use anyhow::bail;
use scope_domain::runs::{resources::JobResources, runner::RunnerMaxConcurrentJobs};
use std::sync::{Arc, Mutex};

type CapacityDetector = dyn Fn() -> anyhow::Result<ResourceCapacity> + Send + Sync;
type UsageDetector = dyn Fn(&str) -> anyhow::Result<ResourceUsage> + Send + Sync;

pub(super) struct ResourceAdmissionCoordinator {
    inner: Arc<CoordinatorInner>,
}

struct CoordinatorInner {
    max_concurrent_jobs: RunnerMaxConcurrentJobs,
    default_limits: ResourceLimits,
    detector: Box<CapacityDetector>,
    usage_detector: Box<UsageDetector>,
    state: Mutex<AdmissionState>,
}

#[derive(Default)]
struct AdmissionState {
    active: Vec<ActiveReservation>,
    next_reservation_id: u64,
}

struct ActiveReservation {
    id: u64,
    container_name: String,
    limits: ResourceLimits,
}

pub(super) struct ResourceAdmissionReservation {
    inner: Arc<CoordinatorInner>,
    id: u64,
    limits: ResourceLimits,
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
                default_limits: limits,
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

    pub(super) fn available_resources(&self) -> anyhow::Result<JobResources> {
        let state = self
            .inner
            .state
            .lock()
            .map_err(|_| anyhow::anyhow!("runner resource admission state is poisoned"))?;
        self.ensure_slot_available(&state)?;
        let active_usage = self.active_usage(&state)?;
        let capacity = (self.inner.detector)()?;
        let resources = capacity.available_job_resources(&active_usage)?;
        capacity.ensure_admission(
            &self.inner.default_limits.with_job_resources(resources),
            &active_usage,
        )?;
        Ok(resources)
    }

    pub(super) fn reserve(
        &self,
        resources: JobResources,
        container_name: String,
    ) -> anyhow::Result<ResourceAdmissionReservation> {
        let mut state = self
            .inner
            .state
            .lock()
            .map_err(|_| anyhow::anyhow!("runner resource admission state is poisoned"))?;
        self.ensure_slot_available(&state)?;
        let limits = self.inner.default_limits.with_job_resources(resources);
        let active_usage = self.active_usage(&state)?;
        (self.inner.detector)()?.ensure_admission(&limits, &active_usage)?;
        let id = state.next_reservation_id;
        state.next_reservation_id = state
            .next_reservation_id
            .checked_add(1)
            .ok_or_else(|| anyhow::anyhow!("runner resource reservation identity overflow"))?;
        state.active.push(ActiveReservation {
            id,
            container_name,
            limits: limits.clone(),
        });
        Ok(ResourceAdmissionReservation {
            inner: Arc::clone(&self.inner),
            id,
            limits,
        })
    }

    fn ensure_slot_available(&self, state: &AdmissionState) -> anyhow::Result<()> {
        debug_assert!(
            state.active.len() <= usize::from(self.inner.max_concurrent_jobs.get()),
            "runner resource admission count exceeds configured slots"
        );
        if state.active.len() >= usize::from(self.inner.max_concurrent_jobs.get()) {
            bail!("all runner resource slots are reserved");
        }
        Ok(())
    }

    fn active_usage<'a>(
        &self,
        state: &'a AdmissionState,
    ) -> anyhow::Result<Vec<ReservedResourceUsage<'a>>> {
        state
            .active
            .iter()
            .map(|reservation| {
                Ok(ReservedResourceUsage {
                    limits: &reservation.limits,
                    usage: (self.inner.usage_detector)(&reservation.container_name)?,
                })
            })
            .collect()
    }
}

impl ResourceAdmissionReservation {
    pub(super) fn limits(&self) -> &ResourceLimits {
        &self.limits
    }
}

impl Drop for ResourceAdmissionReservation {
    fn drop(&mut self) {
        let Ok(mut state) = self.inner.state.lock() else {
            return;
        };
        let index = state
            .active
            .iter()
            .position(|reservation| reservation.id == self.id)
            .expect("active runner resource reservation must be registered");
        state.active.swap_remove(index);
    }
}
