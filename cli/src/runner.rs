use crate::{
    api::{
        AttemptRecoveryLookup, abandon_attempt, api_url, append_attempt_log,
        attach_runner_repository, attempt_heartbeat, attempt_recovery_status,
        attempt_recovery_status_if_active, complete_attempt, detach_runner_repository, get_repo,
        get_runner, register_runner, runner_claim, runner_poll, upgrade_runner_registration,
    },
    login::session_from_cache_or_device,
};
use anyhow::{Context, bail};
use reqwest::blocking::Client;
use scope_api_contract::{
    AppendAttemptLogRequest, AttemptCacheFinalizationOutcome, AttemptConclusionRequest,
    ClaimRunResponse, CompleteAttemptRequest, RegisterRunnerRequest,
    UpgradeRunnerRegistrationRequest,
};
use scope_domain::runs::{
    run::StepState,
    runner::{RUNNER_PROTOCOL_VERSION, RunnerCapabilities},
    step::MAX_RUN_SETUP_FAILURE_MESSAGE_BYTES,
};
use std::{
    path::Path,
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
mod resources;
mod source;
mod step_logs;
mod steps;
mod supervisor;
mod systemd;
mod workspace;
use checkout::checkout_exact_commit;
use config::{
    RunnerConfig, load_runner_config, load_runner_config_from, runner_cache_root,
    runner_config_path, scope_config_home, store_runner_config,
};
#[cfg(test)]
use container::apply_container_limits;
use container::{
    ContainerGuard, DockerCapabilities, JobContainerSpec, configure_job_container_creation,
    configure_source_copy, container_started_at_unix, doctor_local, probe_storage_quota_support,
    require_root_image, stop_container,
};
use image::resolve_container_image;
use management::{parse_repository, print_runner_status};
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
use systemd::{install_systemd_service, print_linger_status};
use workspace::{
    RunnerWorkDir, command_stdout, command_success, command_success_while, runner_work_root,
    unix_now,
};

const LOG_CHUNK_BYTES: usize = 16 * 1024;

pub fn install(name: &str, repository: &str) -> anyhow::Result<()> {
    let (owner, repo) = parse_repository(repository)?;
    doctor_local(true)?;
    let api_url = api_url();
    let client = runner_client()?;
    let session = session_from_cache_or_device(&client, &api_url)?;
    let config_path = runner_config_path()?;
    if config_path.exists() {
        let mut config = load_runner_config_from(&config_path)?;
        if config.api_url != api_url || config.name != name {
            bail!(
                "this machine is already configured as runner {} for {}; remove {} before replacing it",
                config.name,
                config.api_url,
                config_path.display()
            );
        }
        let cache_root = runner_cache_root(config.cache_root.as_deref())?;
        cache::initialize(&cache_root)?;
        let upgraded = upgrade_runner_registration(
            &client,
            &api_url,
            &session.token,
            &config.runner_id,
            &UpgradeRunnerRegistrationRequest {
                version: env!("CARGO_PKG_VERSION").to_string(),
                protocol_version: RUNNER_PROTOCOL_VERSION,
                capabilities: RunnerCapabilities::v1(),
            },
        )?;
        config.secret = upgraded.secret;
        config.cache_root = Some(cache_root);
        store_runner_config(&config_path, &config)?;
        let runner = upgraded.runner;
        let repository_id = get_repo(&client, &api_url, &session.token, owner, repo)?.id;
        if !runner
            .grants
            .iter()
            .any(|grant| grant.active && grant.repository_id == repository_id)
        {
            attach_runner_repository(
                &client,
                &api_url,
                &session.token,
                &config.runner_id,
                owner,
                repo,
                name,
            )?;
        }
        install_systemd_service(&config_path)?;
        println!("✓ Existing runner configuration restored");
        println!("✓ systemd user service installed");
        print_linger_status();
        return Ok(());
    }
    let requested_cache_root = runner_cache_root(None)?;
    cache::initialize(&requested_cache_root)?;
    let registered = register_runner(
        &client,
        &api_url,
        &session.token,
        &RegisterRunnerRequest {
            owner: owner.to_string(),
            repo: repo.to_string(),
            name: name.to_string(),
            version: env!("CARGO_PKG_VERSION").to_string(),
            protocol_version: RUNNER_PROTOCOL_VERSION,
            capabilities: RunnerCapabilities::v1(),
        },
    )?;
    let config = RunnerConfig {
        api_url,
        runner_id: registered.runner.id,
        name: name.to_string(),
        secret: registered.secret,
        cache_root: Some(requested_cache_root),
    };
    if let Err(error) = store_runner_config(&config_path, &config) {
        let _ =
            crate::api::delete_runner(&client, &config.api_url, &session.token, &config.runner_id);
        return Err(error);
    }
    install_systemd_service(&config_path)?;
    println!("✓ Runner secret stored with mode 0600");
    println!("✓ Docker available and test container completed");
    println!("✓ systemd user service installed");
    print_linger_status();
    println!("✓ {name} is registered; the service is starting");
    Ok(())
}

pub fn status() -> anyhow::Result<()> {
    let config = load_runner_config()?;
    let client = runner_client()?;
    let session = session_from_cache_or_device(&client, &config.api_url)?;
    let runner = get_runner(&client, &config.api_url, &session.token, &config.runner_id)?;
    print_runner_status(&config.name, &runner);
    Ok(())
}

pub fn add_repository(repository: &str) -> anyhow::Result<()> {
    let config = load_runner_config()?;
    let (owner, repo) = parse_repository(repository)?;
    let client = runner_client()?;
    let session = session_from_cache_or_device(&client, &config.api_url)?;
    attach_runner_repository(
        &client,
        &config.api_url,
        &session.token,
        &config.runner_id,
        owner,
        repo,
        &config.name,
    )?;
    println!("✓ Repository attached");
    Ok(())
}

pub fn remove_repository(repository: &str) -> anyhow::Result<()> {
    let config = load_runner_config()?;
    let (owner, repo) = parse_repository(repository)?;
    let client = runner_client()?;
    let session = session_from_cache_or_device(&client, &config.api_url)?;
    detach_runner_repository(
        &client,
        &config.api_url,
        &session.token,
        &config.runner_id,
        owner,
        repo,
    )?;
    println!("✓ Repository access revoked");
    Ok(())
}

pub fn doctor() -> anyhow::Result<()> {
    let capabilities = doctor_local(true)?;
    if let Ok(config) = load_runner_config() {
        cache::doctor(&config)?;
        let limits = ResourceLimits::detect()?;
        println!(
            "✓ live resources ({} MiB memory, {:.3} CPU, {} PIDs)",
            limits.memory_bytes / (1024 * 1024),
            limits.cpu_millis as f64 / 1000.0,
            limits.pids
        );
        let client = runner_client()?;
        runner_poll(&client, &config.api_url, &config.secret)?;
        println!("✓ Scope API");
    }
    println!(
        "✓ Docker (writable-layer quotas {})",
        if capabilities.storage_quota_supported {
            "enabled"
        } else {
            "unavailable; best-effort storage guard enabled"
        }
    );
    println!("✓ transient disk");
    println!("✓ cgroups");
    println!("✓ systemd user service");
    Ok(())
}

pub fn list_caches() -> anyhow::Result<()> {
    cache::list(&load_runner_config()?)
}

pub fn prune_caches(all: bool) -> anyhow::Result<()> {
    cache::prune(&load_runner_config()?, all)
}

pub fn daemon(config_path: Option<&Path>) -> anyhow::Result<()> {
    let config = match config_path {
        Some(path) => load_runner_config_from(path)?,
        None => load_runner_config()?,
    };
    let capabilities = doctor_local(false)?;
    let client = runner_client()?;
    eprintln!("Scope runner {} is polling {}", config.name, config.api_url);
    loop {
        resume_interrupted_attempts(&config)?;
        if let Err(error) = cache::admit(&config) {
            eprintln!("Runner admission paused: {error:#}");
            thread::sleep(Duration::from_secs(5));
            continue;
        }
        if let Err(error) = ResourceLimits::detect() {
            eprintln!("Runner admission paused: {error:#}");
            thread::sleep(Duration::from_secs(5));
            continue;
        }
        match runner_poll(&client, &config.api_url, &config.secret) {
            Ok(response) => {
                let Some(offer) = response.run else {
                    continue;
                };
                match runner_claim(&client, &config.api_url, &config.secret, &offer.run_id) {
                    Ok(claim) => run_claim(&config, capabilities, claim),
                    Err(error) => eprintln!("Could not claim {}: {error}", offer.run_id),
                }
            }
            Err(error) => {
                eprintln!("Runner poll failed: {error}");
                thread::sleep(Duration::from_secs(5));
            }
        }
    }
}

fn resume_interrupted_attempts(config: &RunnerConfig) -> anyhow::Result<()> {
    for recovery in recover_runner_state(config)? {
        let attempt_id = recovery.recovery.claim.attempt_id.clone();
        let attempt_token = recovery.recovery.claim.attempt_token.clone();
        if let Err(error) = resume_claim(config, recovery) {
            eprintln!("Could not resume interrupted attempt {attempt_id}: {error:#}");
            if is_conclusion_report_pending(&error) {
                continue;
            }
            if let Ok(client) = runner_client() {
                let _ = abandon_attempt(&client, &config.api_url, &attempt_token, &attempt_id);
            }
        }
    }
    Ok(())
}

fn run_claim(config: &RunnerConfig, capabilities: DockerCapabilities, claim: ClaimRunResponse) {
    if let Err(error) = execute_claim(config, capabilities, &claim) {
        eprintln!(
            "Run {} failed before completion: {error:#}",
            claim.job.run_id
        );
        if is_conclusion_report_pending(&error) {
            return;
        }
        let client = match runner_client() {
            Ok(client) => client,
            Err(client_error) => {
                eprintln!("Could not report failure: {client_error}");
                return;
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
    claim: &ClaimRunResponse,
) -> anyhow::Result<()> {
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
    let step_programs = write_step_programs(&work.path, &claim.job.workflow)?;
    if finish_before_execution(&mut supervisor, &client, config, claim)? {
        return Ok(());
    }
    let phase = Instant::now();
    let limits = ResourceLimits::detect()?;
    let capabilities = if capabilities.storage_quota_supported {
        capabilities
    } else {
        probe_storage_quota_support(&container_image, &limits)?
    };
    let mut caches = cache::PreparedCaches::prepare(config, claim, &container_image)?;
    mark_recovery_caches_attached(&work.path, claim, &caches.volume_names())?;
    log_phase(&claim.attempt_id, "cache_prepare", phase);
    let container_name = format!("scope-{}", claim.attempt_id);
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
            limits: &limits,
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
    let execution_deadline_unix = unix_now().saturating_add(claim.job.workflow.timeout_seconds());
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
        Ok(success) => {
            log_phase(&claim.attempt_id, "steps", phase);
            let cleanup = Instant::now();
            let reusable = cache::is_reusable_after_execution(claim, success);
            if let Err(error) = caches.finish(reusable) {
                eprintln!(
                    "Could not finalize attempt caches; tainted caches were not reused: {error:#}"
                );
                if success && claim.canary_phase.is_some() {
                    cache::finish_canary_ack(
                        &client,
                        config,
                        claim,
                        &mut work,
                        AttemptCacheFinalizationOutcome::Failed,
                    )?;
                }
            } else if success && claim.canary_phase.is_some() {
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
        Ok(success) => {
            let reusable = cache::is_reusable_after_execution(&claim, success);
            if let Err(error) = cache::finalize_volume_names(config, &volumes, reusable) {
                eprintln!("Could not finalize recovered attempt caches: {error:#}");
                if success && claim.canary_phase.is_some() {
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
            } else if success && claim.canary_phase.is_some() {
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
) -> anyhow::Result<bool> {
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
    let container_name = format!("scope-{}", claim.attempt_id);
    let mut container = ContainerGuard::new(container_name.clone());
    let has_pending_stop =
        progress.pending_attempt_abandon || progress.pending_attempt_conclusion.is_some();
    if has_pending_stop
        && drain_pending_stop_logs(config, &claim, &mut work, &mut container, &progress)?
            == PendingStopLogOutcome::AttemptUnavailable
    {
        return Ok(false);
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
        return report_pending_abandon(config, &claim, &mut work).map(|()| false);
    }
    if let Some(conclusion) = progress.pending_attempt_conclusion.take() {
        return report_pending_conclusion(config, &claim, &mut work, conclusion).map(|()| false);
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
                .saturating_add(claim.job.workflow.timeout_seconds());
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
        let succeeded = recovery_status.steps.len() == claim.job.workflow.steps().len()
            && recovery_status
                .steps
                .iter()
                .all(|step| step.state == StepState::Succeeded);
        return Ok(succeeded);
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
    Client::builder()
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
