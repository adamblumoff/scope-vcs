use crate::api::{
    AttemptRecoveryLookup, abandon_attempt, append_attempt_log, attempt_heartbeat,
    attempt_recovery_status, attempt_recovery_status_if_active, complete_attempt, runner_claim,
    runner_poll,
};
use anyhow::{Context, bail};
use reqwest::blocking::Client;
use scope_api_contract::{
    AppendAttemptLogRequest, AttemptCacheFinalizationOutcome, AttemptConclusionRequest,
    ClaimRunResponse, CompleteAttemptRequest,
};
use scope_domain::runs::{
    run::StepState, step::MAX_RUN_SETUP_FAILURE_MESSAGE_BYTES, workflow::WorkflowJob,
};
use std::{
    process::Command,
    thread,
    time::{Duration, Instant},
};

mod cache;
mod checkout;
mod config;
mod container;
mod image;
mod management;
mod resource_admission;
mod resources;
mod source;
mod step_logs;
mod steps;
mod supervisor;
mod systemd;
mod worker_pool;
mod workspace;
use checkout::checkout_exact_commit;
use config::{RunnerConfig, load_runner_config, load_runner_config_from, scope_config_home};
#[cfg(test)]
use container::apply_container_limits;
use container::{
    ContainerGuard, DockerCapabilities, JobContainerSpec, configure_job_container_creation,
    configure_source_copy, container_started_at_unix, doctor_local, job_container_name,
    probe_storage_quota_support, require_root_image, stop_container,
};
use image::resolve_container_image;
#[cfg(test)]
use management::parse_repository;
pub use management::{
    add_repository, doctor, install, list_caches, prune_caches, remove_repository, status,
};
pub use worker_pool::daemon;
#[cfg(test)]
use worker_pool::run_after_recovery;
mod recovery;
use recovery::{
    RecoveryAttempt, RecoveryProgress, mark_recovery_abandon_pending,
    mark_recovery_caches_attached, mark_recovery_conclusion_pending,
    mark_recovery_execution_started, mark_recovery_step_completed, persist_recovery_claim,
    recover_runner_state,
};
use resources::ResourceLimits;
use source::{download_attempt_source, source_download_client};
use step_logs::drain_recovered_step_logs;
#[cfg(test)]
use step_logs::{StableLogDecoder, stable_log_text};
use steps::{
    report_step_conclusion, report_step_conclusion_until_reconciled, run_steps, write_step_programs,
};
use supervisor::{AttemptStopReason, AttemptSupervisor};
#[cfg(test)]
use systemd::systemd_quote_path;
use workspace::{
    RunnerWorkDir, command_stdout, command_success, command_success_while, runner_work_root,
    unix_now,
};

const LOG_CHUNK_BYTES: usize = 16 * 1024;

fn dispatch_job(claim: &ClaimRunResponse) -> anyhow::Result<&WorkflowJob> {
    Ok(&claim.job.definition)
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ExecutionOutcome {
    Succeeded,
    Failed,
    Interrupted,
}

impl ExecutionOutcome {
    fn succeeded(self) -> bool {
        self == Self::Succeeded
    }
}

fn resume_interrupted_attempts(config: &RunnerConfig) -> anyhow::Result<()> {
    let restart_required = run_recovery_tasks(recover_runner_state(config)?, |recovery| {
        let attempt_id = recovery.recovery.claim.attempt_id.clone();
        let attempt_token = recovery.recovery.claim.attempt_token.clone();
        let Err(error) = resume_claim(config, recovery) else {
            return false;
        };
        eprintln!("Could not resume interrupted attempt {attempt_id}: {error:#}");
        if requires_recovery_restart(&error) {
            return true;
        }
        if let Ok(client) = runner_client() {
            let _ = abandon_attempt(&client, &config.api_url, &attempt_token, &attempt_id);
        }
        false
    })
    .into_iter()
    .any(|restart_required| restart_required);
    if restart_required {
        Err(RecoveryRestartRequired.into())
    } else {
        Ok(())
    }
}

fn run_recovery_tasks<T: Send, R: Send>(tasks: Vec<T>, recover: impl Fn(T) -> R + Sync) -> Vec<R> {
    thread::scope(|scope| {
        let recover = &recover;
        tasks
            .into_iter()
            .map(|task| scope.spawn(move || recover(task)))
            .collect::<Vec<_>>()
            .into_iter()
            .map(|worker| {
                worker
                    .join()
                    .unwrap_or_else(|panic| std::panic::resume_unwind(panic))
            })
            .collect()
    })
}

fn run_claim(
    config: &RunnerConfig,
    capabilities: DockerCapabilities,
    limits: &ResourceLimits,
    claim: ClaimRunResponse,
) -> anyhow::Result<()> {
    if let Err(error) = execute_claim(config, capabilities, limits, &claim) {
        eprintln!(
            "Run {} failed before completion: {error:#}",
            claim.job.run_id
        );
        if requires_recovery_restart(&error) {
            return Err(error).context("restart runner to reconcile preserved attempt state");
        }
        let client = match runner_client() {
            Ok(client) => client,
            Err(client_error) => {
                eprintln!("Could not report failure: {client_error}");
                return Ok(());
            }
        };
        if let Err(report_error) = complete_attempt(
            &client,
            &config.api_url,
            &claim.attempt_token,
            &claim.attempt_id,
            &CompleteAttemptRequest {
                conclusion: AttemptConclusionRequest::SetupFailed {
                    exit_code: 1,
                    message: bounded_setup_failure_message(&error),
                },
            },
        ) {
            eprintln!("Could not report failed attempt: {report_error}");
            let _ = abandon_attempt(
                &client,
                &config.api_url,
                &claim.attempt_token,
                &claim.attempt_id,
            );
        }
    }
    Ok(())
}

fn bounded_setup_failure_message(error: &anyhow::Error) -> String {
    let mut message = format!("{error:#}")
        .chars()
        .map(|character| {
            if character.is_control() && !matches!(character, '\n' | '\t') {
                '�'
            } else {
                character
            }
        })
        .collect::<String>();
    if message.len() > MAX_RUN_SETUP_FAILURE_MESSAGE_BYTES {
        let mut end = MAX_RUN_SETUP_FAILURE_MESSAGE_BYTES;
        while !message.is_char_boundary(end) {
            end -= 1;
        }
        message.truncate(end);
    }
    if message.trim().is_empty() {
        "runner setup failed".to_string()
    } else {
        message
    }
}

fn execute_claim(
    config: &RunnerConfig,
    capabilities: DockerCapabilities,
    limits: &ResourceLimits,
    claim: &ClaimRunResponse,
) -> anyhow::Result<()> {
    let job = dispatch_job(claim)?;
    let client = runner_client()?;
    let mut work = RunnerWorkDir::new(&claim.attempt_id)?;
    persist_recovery_claim(&work.path, claim)?;
    let mut supervisor = AttemptSupervisor::start(config.clone(), claim.clone())?;
    cache::admit(config)?;
    supervisor.start_storage_monitor(config.clone(), &claim.attempt_id, &work.path)?;
    let bundle_path = work.path.join("source.bundle");
    let source_client = source_download_client()?;
    let phase = Instant::now();
    download_attempt_source(
        &source_client,
        &config.api_url,
        &claim.attempt_token,
        &claim.attempt_id,
        &claim.job.source_digest,
        &bundle_path,
        || !supervisor.storage_pressure_triggered(),
    )?;
    log_phase(&claim.attempt_id, "source_download", phase);
    if finish_before_execution(&mut supervisor, &client, config, claim)? {
        return Ok(());
    }
    let workspace = work.path.join("workspace");
    let phase = Instant::now();
    checkout_exact_commit(&bundle_path, &workspace, &claim.job.git_oid, || {
        !supervisor.storage_pressure_triggered()
    })?;
    log_phase(&claim.attempt_id, "checkout", phase);
    if finish_before_execution(&mut supervisor, &client, config, claim)? {
        return Ok(());
    }
    let phase = Instant::now();
    let container_image = resolve_container_image(&client, config, claim, || {
        !supervisor.storage_pressure_triggered()
    })?;
    require_root_image(&container_image)?;
    log_phase(&claim.attempt_id, "image_resolution", phase);
    let step_programs = write_step_programs(&work.path, job)?;
    if finish_before_execution(&mut supervisor, &client, config, claim)? {
        return Ok(());
    }
    let phase = Instant::now();
    let capabilities = if capabilities.storage_quota_supported {
        capabilities
    } else {
        probe_storage_quota_support(&container_image, limits)?
    };
    let mut caches = cache::PreparedCaches::prepare(config, claim, &container_image)?;
    mark_recovery_caches_attached(&work.path, claim, &caches.volume_names())?;
    log_phase(&claim.attempt_id, "cache_prepare", phase);
    let container_name = job_container_name(&claim.attempt_id);
    let mut create = Command::new("docker");
    let phase = Instant::now();
    configure_job_container_creation(
        &mut create,
        JobContainerSpec {
            config,
            claim,
            name: &container_name,
            image: &container_image,
            step_programs: &step_programs,
            limits,
            capabilities,
            caches: caches.mounts(),
        },
    );
    command_success(&mut create, "create Docker job container")?;
    let container = ContainerGuard::new(container_name);
    caches.confirm_container(&container.name)?;
    log_phase(&claim.attempt_id, "container_create", phase);
    if finish_before_execution(&mut supervisor, &client, config, claim)? {
        return Ok(());
    }
    let mut copy_source = Command::new("docker");
    configure_source_copy(&mut copy_source, &workspace, &container.name);
    command_success_while(
        &mut copy_source,
        "copy run source into Docker job container",
        || !supervisor.storage_pressure_triggered(),
    )?;
    if finish_before_execution(&mut supervisor, &client, config, claim)? {
        return Ok(());
    }
    let execution_deadline_unix = unix_now().saturating_add(job.timeout_seconds());
    mark_recovery_execution_started(&work.path, claim, execution_deadline_unix)?;
    let phase = Instant::now();
    let result = run_steps(
        config,
        claim,
        &mut work,
        &client,
        0,
        1,
        0,
        false,
        None,
        None,
        execution_deadline_unix,
        &mut supervisor,
        container,
    );
    match result {
        Ok(outcome) => {
            log_phase(&claim.attempt_id, "steps", phase);
            let cleanup = Instant::now();
            let reusable = cache::is_reusable_after_execution(claim.canary_phase, outcome);
            if let Err(error) = caches.finish(reusable) {
                eprintln!(
                    "Could not finalize attempt caches; tainted caches were not reused: {error:#}"
                );
                if outcome.succeeded() && claim.canary_phase.is_some() {
                    cache::finish_canary_ack(
                        &client,
                        config,
                        claim,
                        &mut work,
                        AttemptCacheFinalizationOutcome::Failed,
                    )?;
                }
            } else if outcome.succeeded() && claim.canary_phase.is_some() {
                cache::finish_canary_ack(
                    &client,
                    config,
                    claim,
                    &mut work,
                    AttemptCacheFinalizationOutcome::Succeeded,
                )?;
            }
            log_phase(&claim.attempt_id, "cleanup", cleanup);
            Ok(())
        }
        Err(error) => {
            if !work.cleanup_on_drop {
                caches.preserve();
                return Err(error.context(RecoveryRestartRequired));
            }
            Err(error)
        }
    }
}

fn resume_claim(config: &RunnerConfig, recovery: RecoveryAttempt) -> anyhow::Result<()> {
    let claim = recovery.recovery.claim.clone();
    let work_dir = recovery.work_dir.clone();
    let volumes = recovery.recovery.progress.cache_volumes.clone();
    if let Some(outcome) = recovery
        .recovery
        .progress
        .pending_cache_finalization
        .clone()
    {
        cache::acknowledge_finalization(&attempt_control_client()?, config, &claim, outcome)?;
        std::fs::remove_dir_all(work_dir).context("remove acknowledged canary recovery state")?;
        return Ok(());
    }
    match resume_claim_execution(config, recovery) {
        Ok(outcome) => {
            let reusable = cache::is_reusable_after_execution(claim.canary_phase, outcome);
            if let Err(error) =
                cache::finalize_volume_names(config, &volumes, &claim.attempt_id, reusable)
            {
                eprintln!("Could not finalize recovered attempt caches: {error:#}");
                if outcome.succeeded() && claim.canary_phase.is_some() {
                    let mut work = RunnerWorkDir {
                        path: work_dir.clone(),
                        cleanup_on_drop: false,
                    };
                    cache::finish_canary_ack(
                        &attempt_control_client()?,
                        config,
                        &claim,
                        &mut work,
                        AttemptCacheFinalizationOutcome::Failed,
                    )?;
                }
            } else if outcome.succeeded() && claim.canary_phase.is_some() {
                let mut work = RunnerWorkDir {
                    path: work_dir.clone(),
                    cleanup_on_drop: false,
                };
                cache::finish_canary_ack(
                    &attempt_control_client()?,
                    config,
                    &claim,
                    &mut work,
                    AttemptCacheFinalizationOutcome::Succeeded,
                )?;
            }
            if claim.canary_phase.is_some() {
                std::fs::remove_dir_all(work_dir)
                    .context("remove finalized canary recovery state")?;
            }
            Ok(())
        }
        Err(error) => Err(error),
    }
}

fn resume_claim_execution(
    config: &RunnerConfig,
    recovery: RecoveryAttempt,
) -> anyhow::Result<ExecutionOutcome> {
    let RecoveryAttempt { work_dir, recovery } = recovery;
    let mut work = RunnerWorkDir {
        path: work_dir,
        cleanup_on_drop: true,
    };
    let claim = recovery.claim;
    if claim.canary_phase.is_some() {
        work.preserve();
    }
    let mut progress = recovery.progress;
    let container_name = job_container_name(&claim.attempt_id);
    let mut container = ContainerGuard::new(container_name.clone());
    let has_pending_stop =
        progress.pending_attempt_abandon || progress.pending_attempt_conclusion.is_some();
    if has_pending_stop
        && drain_pending_stop_logs(config, &claim, &mut work, &mut container, &progress)?
            == PendingStopLogOutcome::AttemptUnavailable
    {
        return Ok(ExecutionOutcome::Interrupted);
    }
    if has_pending_stop && let Some(pending) = progress.pending_step_conclusion.take() {
        let control_client = attempt_control_client().map_err(|error| {
            work.preserve();
            eprintln!("Could not resume the completed workflow step: {error:#}");
            ConclusionReportPending
        })?;
        report_step_conclusion(
            &control_client,
            config,
            &claim,
            pending.step_index,
            pending.conclusion.clone(),
        )
        .map_err(|error| {
            work.preserve();
            eprintln!("Could not reconcile the completed workflow step: {error:#}");
            ConclusionReportPending
        })?;
        mark_recovery_step_completed(&work.path, &claim, pending.step_index).map_err(|error| {
            work.preserve();
            eprintln!("Could not commit the reconciled workflow step: {error:#}");
            ConclusionReportPending
        })?;
        advance_recovery_past_replayed_step(&mut progress);
    }
    if progress.pending_attempt_abandon {
        return report_pending_abandon(config, &claim, &mut work)
            .map(|()| ExecutionOutcome::Interrupted);
    }
    if let Some(conclusion) = progress.pending_attempt_conclusion.take() {
        return report_pending_conclusion(config, &claim, &mut work, conclusion)
            .map(|()| ExecutionOutcome::Interrupted);
    }
    let control_client = attempt_control_client()?;
    attempt_heartbeat(
        &control_client,
        &config.api_url,
        &claim.attempt_token,
        &claim.attempt_id,
    )?;
    let recovery_status = attempt_recovery_status(
        &control_client,
        &config.api_url,
        &claim.attempt_token,
        &claim.attempt_id,
    )?;
    let execution_deadline_unix = match progress.execution_deadline_unix {
        Some(deadline) => deadline,
        None => {
            let deadline = container_started_at_unix(&container_name)?
                .saturating_add(dispatch_job(&claim)?.timeout_seconds());
            mark_recovery_execution_started(&work.path, &claim, deadline)?;
            deadline
        }
    };
    let local_next_sequence = progress.next_log_sequence;
    if !matches!(
        recovery_status.next_log_sequence,
        sequence if sequence == local_next_sequence
            || sequence == local_next_sequence.saturating_add(1)
    ) {
        bail!("server and runner log cursors cannot be reconciled");
    }
    let mut supervisor = AttemptSupervisor::start(config.clone(), claim.clone())?;
    supervisor.start_storage_monitor(config.clone(), &claim.attempt_id, &work.path)?;
    supervisor.set_execution_deadline(execution_deadline_unix);

    if let Some(pending) = progress.pending_step_conclusion.take() {
        if let Err(error) = report_step_conclusion_until_reconciled(
            &control_client,
            config,
            &claim,
            &mut work,
            pending.step_index,
            pending.conclusion.clone(),
            &supervisor,
        ) {
            container.preserve();
            return Err(error);
        }
        if let Err(error) = mark_recovery_step_completed(&work.path, &claim, pending.step_index) {
            work.preserve();
            container.preserve();
            eprintln!("Could not commit reconciled step recovery progress: {error:#}");
            return Err(ConclusionReportPending.into());
        }
        advance_recovery_past_replayed_step(&mut progress);
    }

    let recovery_status = attempt_recovery_status(
        &control_client,
        &config.api_url,
        &claim.attempt_token,
        &claim.attempt_id,
    )?;
    let active_step_index = progress
        .active_step_index
        .or_else(|| {
            recovery_status
                .steps
                .iter()
                .find(|step| step.state == StepState::Running)
                .map(|step| step.step_index)
        })
        .or_else(|| {
            recovery_status
                .steps
                .iter()
                .find(|step| step.state == StepState::Pending)
                .map(|step| step.step_index)
        });
    let Some(active_step_index) = active_step_index else {
        let succeeded = recovery_status.steps.len() == dispatch_job(&claim)?.steps().len()
            && recovery_status
                .steps
                .iter()
                .all(|step| step.state == StepState::Succeeded);
        return Ok(if succeeded {
            ExecutionOutcome::Succeeded
        } else {
            ExecutionOutcome::Failed
        });
    };
    let continuing_active_step = recovery_status
        .steps
        .iter()
        .any(|step| step.step_index == active_step_index && step.state == StepState::Running);
    run_steps(
        config,
        &claim,
        &mut work,
        &control_client,
        active_step_index,
        local_next_sequence,
        if continuing_active_step {
            progress.active_step_log_bytes
        } else {
            0
        },
        progress.logs_exhausted,
        progress.pending_log_chunk,
        progress.active_step_nonce,
        execution_deadline_unix,
        &mut supervisor,
        container,
    )
}

fn log_phase(attempt_id: &str, phase: &str, started: Instant) {
    eprintln!(
        "scope_runner_phase attempt_id={} phase={} elapsed_ms={}",
        attempt_id,
        phase,
        started.elapsed().as_millis()
    );
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum PendingStopLogOutcome {
    Drained,
    AttemptUnavailable,
}

fn drain_pending_stop_logs(
    config: &RunnerConfig,
    claim: &ClaimRunResponse,
    work: &mut RunnerWorkDir,
    container: &mut ContainerGuard,
    progress: &RecoveryProgress,
) -> anyhow::Result<PendingStopLogOutcome> {
    let client = attempt_control_client().map_err(|error| {
        work.preserve();
        container.preserve();
        eprintln!("Could not resume final stopped-step log upload: {error:#}");
        ConclusionReportPending
    })?;
    match attempt_recovery_status_if_active(
        &client,
        &config.api_url,
        &claim.attempt_token,
        &claim.attempt_id,
    )
    .map_err(|error| {
        work.preserve();
        container.preserve();
        eprintln!("Could not inspect pending stopped-attempt recovery: {error:#}");
        ConclusionReportPending
    })? {
        AttemptRecoveryLookup::Active(_) => {}
        AttemptRecoveryLookup::Unavailable => {
            return Ok(PendingStopLogOutcome::AttemptUnavailable);
        }
    }
    let Some(step_index) = progress.active_step_index else {
        return Ok(PendingStopLogOutcome::Drained);
    };
    stop_container(&container.name).map_err(|error| {
        work.preserve();
        container.preserve();
        eprintln!("Could not confirm stopped attempt execution ended: {error:#}");
        ConclusionReportPending
    })?;
    drain_recovered_step_logs(
        &client,
        config,
        claim,
        &work.path,
        &container.name,
        step_index,
        progress.next_log_sequence,
        progress.active_step_log_bytes,
        progress.logs_exhausted,
        progress.pending_log_chunk.clone(),
    )
    .map(|()| PendingStopLogOutcome::Drained)
    .map_err(|error| {
        work.preserve();
        container.preserve();
        eprintln!("Could not finish stopped-step log upload: {error:#}");
        ConclusionReportPending.into()
    })
}

fn advance_recovery_past_replayed_step(progress: &mut RecoveryProgress) {
    progress.active_step_index = None;
    progress.active_step_nonce = None;
    progress.active_step_log_bytes = 0;
    progress.pending_log_chunk = None;
    progress.pending_step_conclusion = None;
}

fn conclude_stopped_attempt(
    config: &RunnerConfig,
    claim: &ClaimRunResponse,
    work: &mut RunnerWorkDir,
    reason: AttemptStopReason,
) -> anyhow::Result<()> {
    let conclusion = match reason {
        AttemptStopReason::Cancellation => AttemptConclusionRequest::Canceled,
        AttemptStopReason::TimedOut => AttemptConclusionRequest::TimedOut,
        AttemptStopReason::LeaseLost => {
            mark_recovery_abandon_pending(&work.path, claim)?;
            return report_pending_abandon(config, claim, work);
        }
        AttemptStopReason::None => bail!("attempt stop conclusion requires a stop reason"),
    };
    mark_recovery_conclusion_pending(&work.path, claim, conclusion.clone())?;
    report_pending_conclusion(config, claim, work, conclusion)
}

fn report_pending_abandon(
    config: &RunnerConfig,
    claim: &ClaimRunResponse,
    work: &mut RunnerWorkDir,
) -> anyhow::Result<()> {
    let client = match runner_client() {
        Ok(client) => client,
        Err(error) => {
            work.preserve();
            eprintln!(
                "Could not create the client for abandoning the interrupted attempt: {error:#}"
            );
            return Err(ConclusionReportPending.into());
        }
    };
    let mut last_error = None;
    for _ in 0..3 {
        match abandon_attempt(
            &client,
            &config.api_url,
            &claim.attempt_token,
            &claim.attempt_id,
        ) {
            Ok(()) => return Ok(()),
            Err(error) => {
                last_error = Some(error);
                thread::sleep(Duration::from_secs(1));
            }
        }
    }
    work.preserve();
    let error = last_error.expect("attempt abandonment retry records an error");
    eprintln!("Could not abandon the interrupted attempt: {error:#}");
    Err(ConclusionReportPending.into())
}

#[derive(Debug)]
struct ConclusionReportPending;

impl std::fmt::Display for ConclusionReportPending {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("attempt conclusion is persisted locally for retry")
    }
}

impl std::error::Error for ConclusionReportPending {}

fn is_conclusion_report_pending(error: &anyhow::Error) -> bool {
    error
        .chain()
        .any(|cause| cause.is::<ConclusionReportPending>())
}

#[derive(Debug)]
struct RecoveryRestartRequired;

impl std::fmt::Display for RecoveryRestartRequired {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("preserved attempt state requires runner restart")
    }
}

impl std::error::Error for RecoveryRestartRequired {}

fn requires_recovery_restart(error: &anyhow::Error) -> bool {
    is_conclusion_report_pending(error)
        || error
            .chain()
            .any(|cause| cause.is::<RecoveryRestartRequired>())
}

fn report_pending_conclusion(
    config: &RunnerConfig,
    claim: &ClaimRunResponse,
    work: &mut RunnerWorkDir,
    conclusion: AttemptConclusionRequest,
) -> anyhow::Result<()> {
    let client = match runner_client() {
        Ok(client) => client,
        Err(error) => {
            work.preserve();
            eprintln!(
                "Could not create the client for reporting the persisted conclusion: {error:#}"
            );
            return Err(ConclusionReportPending.into());
        }
    };
    let request = CompleteAttemptRequest { conclusion };
    let mut last_error = None;
    for _ in 0..3 {
        match complete_attempt(
            &client,
            &config.api_url,
            &claim.attempt_token,
            &claim.attempt_id,
            &request,
        ) {
            Ok(_) => return Ok(()),
            Err(error) => {
                last_error = Some(error);
                thread::sleep(Duration::from_secs(1));
            }
        }
    }
    work.preserve();
    let error = last_error.expect("conclusion retry records an error");
    eprintln!("Could not report the persisted attempt conclusion: {error:#}");
    Err(ConclusionReportPending.into())
}

fn finish_before_execution(
    supervisor: &mut AttemptSupervisor,
    client: &Client,
    config: &RunnerConfig,
    claim: &ClaimRunResponse,
) -> anyhow::Result<bool> {
    match supervisor.reason() {
        AttemptStopReason::None => {}
        AttemptStopReason::Cancellation => {
            let _ = supervisor.finish();
            complete_canceled(client, config, claim)?;
            return Ok(true);
        }
        AttemptStopReason::LeaseLost => {
            let _ = supervisor.finish();
            abandon_attempt(
                client,
                &config.api_url,
                &claim.attempt_token,
                &claim.attempt_id,
            )?;
            return Ok(true);
        }
        AttemptStopReason::TimedOut => {
            let _ = supervisor.finish();
            bail!("attempt timed out before execution started")
        }
    }
    if supervisor.storage_pressure_triggered() {
        let _ = supervisor.finish();
        bail!("runner storage crossed its emergency floor before execution started");
    }
    Ok(false)
}

fn append_log_with_retry(
    client: &Client,
    config: &RunnerConfig,
    claim: &ClaimRunResponse,
    step_index: u32,
    sequence: u64,
    text: String,
) -> anyhow::Result<bool> {
    let request = AppendAttemptLogRequest {
        step_index,
        sequence,
        text,
    };
    let mut last_error = None;
    for _ in 0..3 {
        match append_attempt_log(
            client,
            &config.api_url,
            &claim.attempt_token,
            &claim.attempt_id,
            &request,
        ) {
            Ok(accepted) => return Ok(accepted),
            Err(error) => {
                last_error = Some(error);
                thread::sleep(Duration::from_secs(1));
            }
        }
    }
    Err(last_error.expect("log retry records an error"))
}

fn complete_canceled(
    client: &Client,
    config: &RunnerConfig,
    claim: &ClaimRunResponse,
) -> anyhow::Result<()> {
    complete_attempt(
        client,
        &config.api_url,
        &claim.attempt_token,
        &claim.attempt_id,
        &CompleteAttemptRequest {
            conclusion: AttemptConclusionRequest::Canceled,
        },
    )?;
    Ok(())
}

fn runner_client() -> anyhow::Result<Client> {
    crate::api::http_client_builder()
        .timeout(Duration::from_secs(35))
        .build()
        .context("build runner HTTP client")
}

fn attempt_control_client() -> anyhow::Result<Client> {
    Client::builder()
        .timeout(Duration::from_secs(10))
        .build()
        .context("build attempt control HTTP client")
}

#[cfg(test)]
mod tests;
