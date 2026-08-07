use super::{
    DockerCapabilities, ResourceLimits, RunnerConfig, cache, load_runner_config,
    load_runner_config_from, resume_interrupted_attempts, run_claim, runner_claim, runner_client,
    runner_poll,
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
    run_after_recovery(
        slots,
        {
            let config = config.clone();
            move || resume_interrupted_attempts(&config)
        },
        move |slot| runner_slot(config.clone(), capabilities, limits.clone(), slot),
    )
}

fn runner_slot(
    config: RunnerConfig,
    capabilities: DockerCapabilities,
    limits: ResourceLimits,
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
                match runner_claim(
                    &client,
                    &config.api_url,
                    &config.secret,
                    &offer.run_id,
                    &offer.job_key,
                ) {
                    Ok(claim) => run_claim(&config, capabilities, &limits, claim),
                    Err(error) => {
                        eprintln!(
                            "Runner slot {slot} could not claim {}: {error}",
                            offer.run_id
                        )
                    }
                }
            }
            Err(error) => {
                eprintln!("Runner slot {slot} poll failed: {error}");
                thread::sleep(Duration::from_secs(5));
            }
        }
    }
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
