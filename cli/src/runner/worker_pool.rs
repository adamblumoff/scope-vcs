use super::{
    DockerCapabilities, ResourceLimits, RunnerConfig, cache, job_container_name,
    load_runner_config, load_runner_config_from,
    resource_admission::{ResourceAdmissionCoordinator, ResourceAdmissionReservation},
    resume_interrupted_attempts, run_claim, runner_claim, runner_client, runner_poll,
};
use anyhow::{Context, bail};
use scope_domain::runs::runner::RunnerMaxConcurrentJobs;
use std::{
    panic::{AssertUnwindSafe, catch_unwind},
    path::Path,
    sync::{Arc, mpsc},
    thread,
    time::Duration,
};

pub fn daemon(config_path: Option<&Path>) -> anyhow::Result<()> {
    let config = match config_path {
        Some(path) => load_runner_config_from(path)?,
        None => load_runner_config()?,
    };
    let (capabilities, limits) = super::doctor_local(false, config.max_concurrent_jobs)?;
    eprintln!(
        "Scope runner {} is polling {} with {} job slot(s)",
        config.name,
        config.api_url,
        config.max_concurrent_jobs.get()
    );
    let slots = config.max_concurrent_jobs;
    let admission = ResourceAdmissionCoordinator::new(slots, limits);
    run_after_recovery(
        slots,
        {
            let config = config.clone();
            move || resume_interrupted_attempts(&config)
        },
        move |slot| runner_slot(config.clone(), capabilities, admission.clone(), slot),
    )
}

fn runner_slot(
    config: RunnerConfig,
    capabilities: DockerCapabilities,
    admission: ResourceAdmissionCoordinator,
    slot: u8,
) -> anyhow::Result<()> {
    let client = runner_client()?;
    loop {
        if let Err(error) = cache::admit(&config) {
            eprintln!("Runner slot {slot} admission paused: {error:#}");
            thread::sleep(Duration::from_secs(5));
            continue;
        }
        match runner_poll(&client, &config.api_url, &config.secret) {
            Ok(response) => {
                let Some(offer) = response.run else {
                    continue;
                };
                let admitted = match admit_and_claim(
                    &admission,
                    || {
                        runner_claim(
                            &client,
                            &config.api_url,
                            &config.secret,
                            &offer.run_id,
                            &offer.job_key,
                        )
                    },
                    |claim| job_container_name(&claim.attempt_id),
                ) {
                    Ok(Ok(admitted)) => admitted,
                    Ok(Err(error)) => {
                        eprintln!(
                            "Runner slot {slot} could not claim {}: {error}",
                            offer.run_id
                        );
                        continue;
                    }
                    Err(error) => {
                        eprintln!("Runner slot {slot} resource admission paused: {error:#}");
                        thread::sleep(Duration::from_secs(5));
                        continue;
                    }
                };
                admitted.run(|limits, claim| run_claim(&config, capabilities, limits, claim))?;
            }
            Err(error) => {
                eprintln!("Runner slot {slot} poll failed: {error}");
                thread::sleep(Duration::from_secs(5));
            }
        }
    }
}

struct AdmittedClaim<T> {
    claim: T,
    reservation: ResourceAdmissionReservation,
}

impl<T> AdmittedClaim<T> {
    fn run<R>(self, run: impl FnOnce(&ResourceLimits, T) -> R) -> R {
        let Self { claim, reservation } = self;
        run(reservation.limits(), claim)
    }
}

fn admit_and_claim<T, E, C, N>(
    admission: &ResourceAdmissionCoordinator,
    claim: C,
    container_name: N,
) -> anyhow::Result<Result<AdmittedClaim<T>, E>>
where
    C: FnOnce() -> Result<T, E>,
    N: FnOnce(&T) -> String,
{
    let mut reservation = admission.reserve()?;
    let claim = match claim() {
        Ok(claim) => claim,
        Err(error) => return Ok(Err(error)),
    };
    reservation.activate(container_name(&claim));
    Ok(Ok(AdmittedClaim { claim, reservation }))
}

pub(super) fn run_after_recovery<R, W>(
    slots: RunnerMaxConcurrentJobs,
    recover: R,
    worker: W,
) -> anyhow::Result<()>
where
    R: FnOnce() -> anyhow::Result<()>,
    W: Fn(u8) -> anyhow::Result<()> + Send + Sync + 'static,
{
    recover()?;
    let worker = Arc::new(worker);
    let (result_sender, result_receiver) = mpsc::channel();
    for slot in 1..=slots.get() {
        let worker = Arc::clone(&worker);
        let result_sender = result_sender.clone();
        thread::Builder::new()
            .name(format!("scope-runner-slot-{slot}"))
            .spawn(move || {
                let outcome = catch_unwind(AssertUnwindSafe(|| worker(slot)));
                let _ = result_sender.send((slot, outcome));
            })
            .context("start runner slot worker")?;
    }
    drop(result_sender);
    let (slot, outcome) = result_receiver
        .recv()
        .context("runner slot result channel closed")?;
    match outcome {
        Ok(Ok(())) => bail!("runner slot {slot} stopped unexpectedly"),
        Ok(Err(error)) => Err(error).with_context(|| format!("runner slot {slot} failed")),
        Err(_) => bail!("runner slot {slot} panicked"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runner::resources::{ResourceCapacity, ResourceUsage};
    use std::{
        collections::VecDeque,
        sync::{
            Barrier, Mutex,
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

        let first = admit_and_claim(
            &admission,
            || Ok::<_, ()>("first"),
            |claim| claim.to_string(),
        )
        .unwrap()
        .unwrap();
        let second = admit_and_claim(
            &admission,
            || Ok::<_, ()>("second"),
            |claim| claim.to_string(),
        )
        .unwrap()
        .unwrap();
        assert!(
            admit_and_claim(
                &admission,
                || Ok::<_, ()>("third"),
                |claim| claim.to_string()
            )
            .is_err()
        );
        assert_eq!(first.reservation.limits(), &limits);
        assert_eq!(second.reservation.limits(), &limits);
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

        let _first = admit_and_claim(
            &admission,
            || Ok::<_, ()>("first"),
            |claim| claim.to_string(),
        )
        .unwrap()
        .unwrap();
        let error = match admit_and_claim(
            &admission,
            || Ok::<_, ()>("second"),
            |claim| claim.to_string(),
        ) {
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

        let _first = admit_and_claim(
            &admission,
            || Ok::<_, ()>("first"),
            |claim| claim.to_string(),
        )
        .unwrap()
        .unwrap();
        let error = match admit_and_claim(
            &admission,
            || Ok::<_, ()>("second"),
            |claim| claim.to_string(),
        ) {
            Err(error) => error,
            Ok(_) => panic!("CPU overcommit admitted another claim"),
        };
        assert!(error.to_string().contains("CPU headroom"));
    }

    #[test]
    fn concurrent_offers_cannot_share_one_free_reservation() {
        let slots = RunnerMaxConcurrentJobs::new(2).unwrap();
        let limits = limits();
        let capacity = ResourceCapacity::exactly_provisioned(&limits, 1);
        let admission = ResourceAdmissionCoordinator::for_test(
            slots,
            limits,
            move || Ok(capacity),
            |_| Ok(ResourceUsage::default()),
        );
        let start = Arc::new(Barrier::new(3));
        let release = Arc::new(Barrier::new(2));
        let (sender, receiver) = std::sync::mpsc::channel();
        let mut workers = Vec::new();
        for _ in 0..2 {
            let admission = admission.clone();
            let start = Arc::clone(&start);
            let release = Arc::clone(&release);
            let sender = sender.clone();
            workers.push(thread::spawn(move || {
                start.wait();
                match admit_and_claim(
                    &admission,
                    || {
                        sender.send(true).unwrap();
                        release.wait();
                        Ok::<_, ()>(())
                    },
                    |_| "container".to_string(),
                ) {
                    Ok(Ok(admitted)) => {
                        drop(admitted);
                    }
                    Err(_) => sender.send(false).unwrap(),
                    Ok(Err(())) => unreachable!(),
                }
            }));
        }
        drop(sender);
        start.wait();
        let mut outcomes = vec![receiver.recv().unwrap(), receiver.recv().unwrap()];
        outcomes.sort_unstable();
        assert_eq!(outcomes, [false, true]);
        release.wait();
        for worker in workers {
            worker.join().unwrap();
        }
    }

    #[test]
    fn claim_failure_and_panicked_run_release_their_reservations() {
        let slots = RunnerMaxConcurrentJobs::new(1).unwrap();
        let limits = limits();
        let capacity = ResourceCapacity::exactly_provisioned(&limits, slots.get());
        let admission = ResourceAdmissionCoordinator::for_test(
            slots,
            limits,
            move || Ok(capacity),
            |_| Ok(ResourceUsage::default()),
        );

        let failed = admit_and_claim(
            &admission,
            || Err::<(), _>("claim failed"),
            |_| unreachable!(),
        )
        .unwrap();
        assert!(matches!(failed, Err("claim failed")));

        let panicked = catch_unwind(AssertUnwindSafe(|| {
            admit_and_claim(
                &admission,
                || Ok::<_, ()>("claim"),
                |claim| claim.to_string(),
            )
            .unwrap()
            .unwrap()
            .run(|limits, claim| -> () {
                assert_eq!(claim, "claim");
                assert_eq!(limits.cpu_millis, 1000);
                panic!("run panic");
            })
        }));
        assert!(panicked.is_err());

        assert!(
            admit_and_claim(
                &admission,
                || Ok::<_, ()>("next"),
                |claim| claim.to_string()
            )
            .is_ok()
        );
    }
}
