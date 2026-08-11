use super::{
    DockerCapabilities, RunnerConfig, abandon_attempt, cache, job_container_name,
    load_runner_config, load_runner_config_from, resource_admission::ResourceAdmissionCoordinator,
    resume_interrupted_attempts, run_claim, runner_client, runner_poll,
};
use anyhow::{Context, bail};
use scope_api_contract::RunnerPollRequest;
use scope_domain::runs::runner::RunnerMaxConcurrentJobs;
use std::{
    panic::{AssertUnwindSafe, catch_unwind},
    path::Path,
    sync::mpsc,
    thread,
    time::Duration,
};

pub fn daemon(config_path: Option<&Path>) -> anyhow::Result<()> {
    let config = match config_path {
        Some(path) => load_runner_config_from(path)?,
        None => load_runner_config()?,
    };
    resume_interrupted_attempts(&config)?;
    let (capabilities, limits) = super::doctor_local(false, config.max_concurrent_jobs)?;
    eprintln!(
        "Scope runner {} is dispatching from {} with {} job slot(s)",
        config.name,
        config.api_url,
        config.max_concurrent_jobs.get()
    );
    let admission = ResourceAdmissionCoordinator::new(config.max_concurrent_jobs, limits);
    run_dispatch_coordinator(config, capabilities, admission)
}

fn run_dispatch_coordinator(
    config: RunnerConfig,
    capabilities: DockerCapabilities,
    admission: ResourceAdmissionCoordinator,
) -> anyhow::Result<()> {
    let client = runner_client()?;
    let mut workers = JobWorkers::new(config.max_concurrent_jobs);
    loop {
        workers.wait_for_capacity()?;
        if let Err(error) = cache::admit(&config) {
            eprintln!("Runner dispatch admission paused: {error:#}");
            workers.wait_for_change(Duration::from_secs(5))?;
            continue;
        }
        let available_resources = match admission.available_resources() {
            Ok(resources) => resources,
            Err(error) => {
                eprintln!("Runner resource admission paused: {error:#}");
                workers.wait_for_change(Duration::from_secs(5))?;
                continue;
            }
        };
        let response = match runner_poll(
            &client,
            &config.api_url,
            &config.secret,
            &RunnerPollRequest {
                available_resources,
            },
        ) {
            Ok(response) => response,
            Err(error) => {
                eprintln!("Runner poll failed: {error}");
                workers.wait_for_change(Duration::from_secs(5))?;
                continue;
            }
        };
        let Some(claim) = response.claim else {
            continue;
        };
        let reservation = match admission.reserve(
            claim.job.definition.resources(),
            job_container_name(&claim.attempt_id),
        ) {
            Ok(reservation) => reservation,
            Err(error) => {
                abandon_attempt(
                    &client,
                    &config.api_url,
                    &claim.attempt_token,
                    &claim.attempt_id,
                )
                .context("abandon atomically claimed job rejected by local admission")?;
                eprintln!(
                    "Runner rejected claimed attempt {} after host capacity changed: {error:#}",
                    claim.attempt_id
                );
                workers.wait_for_change(Duration::from_secs(5))?;
                continue;
            }
        };
        let limits = reservation.limits().clone();
        let attempt_id = claim.attempt_id.clone();
        let worker_config = config.clone();
        workers.spawn(attempt_id, move || {
            let _reservation = reservation;
            run_claim(&worker_config, capabilities, &limits, claim)
        })?;
    }
}

struct WorkerCompletion {
    attempt_id: String,
    outcome: std::thread::Result<anyhow::Result<()>>,
}

pub(super) struct JobWorkers {
    max_active: usize,
    active: usize,
    completion_sender: mpsc::Sender<WorkerCompletion>,
    completion_receiver: mpsc::Receiver<WorkerCompletion>,
}

impl JobWorkers {
    pub(super) fn new(max_active: RunnerMaxConcurrentJobs) -> Self {
        let (completion_sender, completion_receiver) = mpsc::channel();
        Self {
            max_active: usize::from(max_active.get()),
            active: 0,
            completion_sender,
            completion_receiver,
        }
    }

    pub(super) fn spawn(
        &mut self,
        attempt_id: String,
        run: impl FnOnce() -> anyhow::Result<()> + Send + 'static,
    ) -> anyhow::Result<()> {
        assert!(self.active < self.max_active, "runner worker capacity exceeded");
        let completion_sender = self.completion_sender.clone();
        let thread_name = format!("scope-runner-{attempt_id}");
        thread::Builder::new()
            .name(thread_name)
            .spawn(move || {
                let outcome = catch_unwind(AssertUnwindSafe(run));
                let _ = completion_sender.send(WorkerCompletion {
                    attempt_id,
                    outcome,
                });
            })
            .context("start runner job worker")?;
        self.active += 1;
        Ok(())
    }

    pub(super) fn wait_for_capacity(&mut self) -> anyhow::Result<()> {
        while let Ok(completion) = self.completion_receiver.try_recv() {
            self.finish(completion)?;
        }
        while self.active >= self.max_active {
            self.receive()?;
        }
        Ok(())
    }

    pub(super) fn wait_for_all(&mut self) -> anyhow::Result<()> {
        while self.active > 0 {
            self.receive()?;
        }
        Ok(())
    }

    fn receive(&mut self) -> anyhow::Result<()> {
        let completion = self
            .completion_receiver
            .recv()
            .context("runner job completion channel closed")?;
        self.finish(completion)
    }

    fn wait_for_change(&mut self, timeout: Duration) -> anyhow::Result<()> {
        if self.active == 0 {
            thread::sleep(timeout);
            return Ok(());
        }
        match self.completion_receiver.recv_timeout(timeout) {
            Ok(completion) => self.finish(completion),
            Err(mpsc::RecvTimeoutError::Timeout) => Ok(()),
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                bail!("runner job completion channel closed")
            }
        }
    }

    fn finish(&mut self, completion: WorkerCompletion) -> anyhow::Result<()> {
        assert!(self.active > 0, "runner worker count is inconsistent");
        self.active -= 1;
        match completion.outcome {
            Ok(Ok(())) => Ok(()),
            Ok(Err(error)) => {
                Err(error).with_context(|| format!("runner attempt {} failed", completion.attempt_id))
            }
            Err(_) => bail!("runner attempt {} panicked", completion.attempt_id),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runner::{
        resource_admission::ResourceAdmissionReservation,
        resources::{ResourceCapacity, ResourceLimits, ResourceUsage},
    };
    use scope_domain::runs::resources::JobResources;
    use std::{
        collections::VecDeque,
        sync::{
            Arc, Mutex,
            atomic::{AtomicUsize, Ordering},
        },
    };

    fn limits() -> ResourceLimits {
        ResourceLimits {
            memory_bytes: 1024 * 1024 * 1024,
            cpu_millis: 1000,
            pids: 256,
            storage_bytes: 4 * 1024 * 1024 * 1024,
        }
    }

    fn resources(limits: &ResourceLimits) -> JobResources {
        JobResources::new(limits.cpu_millis, limits.memory_bytes).unwrap()
    }

    fn admit(
        admission: &ResourceAdmissionCoordinator,
        resources: JobResources,
        container_name: &str,
    ) -> ResourceAdmissionReservation {
        admission
            .reserve(resources, container_name.to_string())
            .unwrap()
    }

    #[test]
    fn poll_capacity_tracks_unconsumed_active_reservations() {
        let slots = RunnerMaxConcurrentJobs::new(2).unwrap();
        let limits = limits();
        let exact = ResourceCapacity::exactly_provisioned(&limits, slots.get());
        let after_first = exact.after_active_usage(&limits);
        let capacities = Arc::new(Mutex::new(VecDeque::from([exact, exact, after_first])));
        let admission = ResourceAdmissionCoordinator::for_test(
            slots,
            limits.clone(),
            {
                let capacities = Arc::clone(&capacities);
                move || Ok(capacities.lock().unwrap().pop_front().unwrap())
            },
            {
                let limits = limits.clone();
                move |_| {
                    Ok(ResourceUsage {
                        memory_bytes: limits.memory_bytes,
                        pids: limits.pids,
                        storage_bytes: limits.storage_bytes,
                    })
                }
            },
        );

        assert_eq!(
            admission.available_resources().unwrap(),
            JobResources::new(2_000, 2 * 1024 * 1024 * 1024).unwrap()
        );
        let _first = admit(&admission, resources(&limits), "first");
        assert_eq!(
            admission.available_resources().unwrap(),
            JobResources::new(1_000, 1024 * 1024 * 1024).unwrap()
        );
    }

    #[test]
    fn claimed_job_receives_its_exact_cpu_and_memory_limits() {
        let slots = RunnerMaxConcurrentJobs::new(1).unwrap();
        let defaults = limits();
        let requested = JobResources::new(1_500, 2 * 1024 * 1024 * 1024).unwrap();
        let requested_limits = defaults.with_job_resources(requested);
        let capacity = ResourceCapacity::exactly_provisioned(&requested_limits, 1);
        let admission = ResourceAdmissionCoordinator::for_test(
            slots,
            defaults.clone(),
            move || Ok(capacity),
            |_| Ok(ResourceUsage::default()),
        );

        let reservation = admission.reserve(requested, "exact".to_string()).unwrap();
        assert_eq!(reservation.limits().cpu_millis, requested.cpu_millis());
        assert_eq!(
            reservation.limits().memory_bytes,
            requested.memory_bytes()
        );
        assert_eq!(reservation.limits().pids, defaults.pids);
        assert_eq!(reservation.limits().storage_bytes, defaults.storage_bytes);
    }

    #[test]
    fn exactly_provisioned_two_slot_host_admits_two_claims_and_never_a_third() {
        let slots = RunnerMaxConcurrentJobs::new(2).unwrap();
        let limits = limits();
        let exact = ResourceCapacity::exactly_provisioned(&limits, slots.get());
        let after_first = exact.after_active_usage(&limits);
        let capacities = Arc::new(Mutex::new(VecDeque::from([exact, after_first])));
        let detections = Arc::new(AtomicUsize::new(0));
        let admission = ResourceAdmissionCoordinator::for_test(
            slots,
            limits.clone(),
            {
                let capacities = Arc::clone(&capacities);
                let detections = Arc::clone(&detections);
                move || {
                    detections.fetch_add(1, Ordering::SeqCst);
                    Ok(capacities.lock().unwrap().pop_front().unwrap())
                }
            },
            {
                let limits = limits.clone();
                move |container_name| {
                    assert_eq!(container_name, "first");
                    Ok(ResourceUsage {
                        memory_bytes: limits.memory_bytes,
                        pids: limits.pids,
                        storage_bytes: limits.storage_bytes,
                    })
                }
            },
        );

        let resources = resources(&limits);
        let first = admit(&admission, resources, "first");
        let second = admit(&admission, resources, "second");
        assert!(admission.reserve(resources, "third".to_string()).is_err());
        assert_eq!(first.limits(), &limits);
        assert_eq!(second.limits(), &limits);
        assert_eq!(detections.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn active_ceiling_reserves_memory_not_yet_used_by_the_container() {
        let slots = RunnerMaxConcurrentJobs::new(2).unwrap();
        let limits = limits();
        let exact = ResourceCapacity::exactly_provisioned(&limits, slots.get());
        // The first container uses 100 MiB while an external process consumes
        // the other 900 MiB of its 1 GiB ceiling.
        let shrunken = exact.after_active_usage(&limits);
        let capacities = Arc::new(Mutex::new(VecDeque::from([exact, shrunken])));
        let admission = ResourceAdmissionCoordinator::for_test(
            slots,
            limits.clone(),
            {
                let capacities = Arc::clone(&capacities);
                move || Ok(capacities.lock().unwrap().pop_front().unwrap())
            },
            {
                let limits = limits.clone();
                move |container_name| {
                    assert_eq!(container_name, "first");
                    Ok(ResourceUsage {
                        memory_bytes: 100 * 1024 * 1024,
                        pids: limits.pids,
                        storage_bytes: limits.storage_bytes,
                    })
                }
            },
        );

        let resources = resources(&limits);
        let _first = admit(&admission, resources, "first");
        let error = match admission.reserve(resources, "second".to_string()) {
            Err(error) => error,
            Ok(_) => panic!("shrunken headroom admitted another claim"),
        };
        assert!(error.to_string().contains("memory headroom"));
    }

    #[test]
    fn cpu_admission_always_reserves_the_full_active_ceiling() {
        let slots = RunnerMaxConcurrentJobs::new(2).unwrap();
        let limits = limits();
        let exact = ResourceCapacity::exactly_provisioned(&limits, slots.get());
        let cpu_short = exact.after_active_usage(&limits).shrink_cpu_by(1);
        let capacities = Arc::new(Mutex::new(VecDeque::from([exact, cpu_short])));
        let admission = ResourceAdmissionCoordinator::for_test(
            slots,
            limits.clone(),
            {
                let capacities = Arc::clone(&capacities);
                move || Ok(capacities.lock().unwrap().pop_front().unwrap())
            },
            {
                let limits = limits.clone();
                move |_| {
                    Ok(ResourceUsage {
                        memory_bytes: limits.memory_bytes,
                        pids: limits.pids,
                        storage_bytes: limits.storage_bytes,
                    })
                }
            },
        );

        let resources = resources(&limits);
        let _first = admit(&admission, resources, "first");
        let error = match admission.reserve(resources, "second".to_string()) {
            Err(error) => error,
            Ok(_) => panic!("CPU overcommit admitted another claim"),
        };
        assert!(error.to_string().contains("CPU headroom"));
    }

    #[test]
    fn dropped_and_panicked_runs_release_their_reservations() {
        let slots = RunnerMaxConcurrentJobs::new(1).unwrap();
        let limits = limits();
        let capacity = ResourceCapacity::exactly_provisioned(&limits, slots.get());
        let resources = resources(&limits);
        let admission = ResourceAdmissionCoordinator::for_test(
            slots,
            limits,
            move || Ok(capacity),
            |_| Ok(ResourceUsage::default()),
        );

        drop(admission.reserve(resources, "dropped".to_string()).unwrap());

        let panicked = catch_unwind(AssertUnwindSafe(|| {
            let reservation = admit(&admission, resources, "claim");
            assert_eq!(reservation.limits().cpu_millis, 1000);
            panic!("run panic");
        }));
        assert!(panicked.is_err());

        assert!(admission.reserve(resources, "next".to_string()).is_ok());
    }
}
