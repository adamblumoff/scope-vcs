use crate::{
    api::{
        abandon_attempt, api_url, append_attempt_log, attach_runner_repository, attempt_heartbeat,
        attempt_recovery_status, attempt_source, attempt_start, complete_attempt,
        detach_runner_repository, get_repo, get_runner, register_runner, runner_claim, runner_poll,
    },
    login::session_from_cache_or_device,
};
use anyhow::{Context, bail};
use reqwest::blocking::Client;
use scope_api_contract::{
    AppendAttemptLogRequest, AttemptConclusionRequest, ClaimRunResponse, CompleteAttemptRequest,
    RegisterRunnerRequest, RunnerResponse,
};
use scope_domain::runs::runner::{RUNNER_PROTOCOL_VERSION, RunnerCapabilities};
use sha2::{Digest, Sha256};
use std::{
    env, fs,
    io::{BufReader, Read, Write},
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    sync::mpsc,
    thread,
    time::Duration,
};

mod container;
mod image;
mod supervisor;
mod systemd;
#[cfg(test)]
use container::apply_container_limits;
use container::{
    ContainerGuard, configure_job_container_creation, container_finished_at_unix,
    container_is_running, container_started_at_unix, doctor_local, recovered_container_exit_code,
};
use image::resolve_container_image;
mod recovery;
#[cfg(test)]
use recovery::RecoveryProgress;
use recovery::{
    RecoveryAttempt, mark_recovery_execution_started, persist_recovery_claim, recover_runner_state,
    update_recovery_log_sequence,
};
use supervisor::{AttemptStopReason, AttemptSupervisor};
#[cfg(test)]
use systemd::systemd_quote_path;
use systemd::{install_systemd_service, print_linger_status};

const LOG_CHUNK_BYTES: usize = 16 * 1024;
const MAX_SOURCE_BUNDLE_BYTES: u64 = 128 * 1024 * 1024;

#[derive(Clone, Debug, serde::Deserialize, serde::Serialize)]
struct RunnerConfig {
    api_url: String,
    runner_id: String,
    name: String,
    secret: String,
    storage_quota_supported: bool,
}

pub fn install(name: &str, repository: &str) -> anyhow::Result<()> {
    let (owner, repo) = parse_repository(repository)?;
    let capabilities = doctor_local(true)?;
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
        let runner = get_runner(&client, &api_url, &session.token, &config.runner_id)?;
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
        if config.storage_quota_supported != capabilities.storage_quota_supported {
            config.storage_quota_supported = capabilities.storage_quota_supported;
            store_runner_config(&config_path, &config)?;
        }
        install_systemd_service(&config_path)?;
        println!("✓ Existing runner configuration restored");
        println!("✓ systemd user service installed");
        print_linger_status();
        return Ok(());
    }
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
        storage_quota_supported: capabilities.storage_quota_supported,
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
    doctor_local(true)?;
    if let Ok(config) = load_runner_config() {
        let client = runner_client()?;
        runner_poll(&client, &config.api_url, &config.secret)?;
        println!("✓ Scope API");
    }
    println!("✓ Docker");
    println!("✓ disk");
    println!("✓ cgroups");
    println!("✓ systemd user service");
    Ok(())
}

pub fn daemon(config_path: Option<&Path>) -> anyhow::Result<()> {
    let config = match config_path {
        Some(path) => load_runner_config_from(path)?,
        None => load_runner_config()?,
    };
    for recovery in recover_runner_state(&config)? {
        let attempt_id = recovery.recovery.claim.attempt_id.clone();
        let attempt_token = recovery.recovery.claim.attempt_token.clone();
        if let Err(error) = resume_claim(&config, recovery) {
            eprintln!("Could not resume interrupted attempt {attempt_id}: {error:#}");
            if let Ok(client) = runner_client() {
                let _ = abandon_attempt(&client, &config.api_url, &attempt_token, &attempt_id);
            }
        }
    }
    let client = runner_client()?;
    eprintln!("Scope runner {} is polling {}", config.name, config.api_url);
    loop {
        match runner_poll(&client, &config.api_url, &config.secret) {
            Ok(response) => {
                let Some(offer) = response.run else {
                    continue;
                };
                match runner_claim(&client, &config.api_url, &config.secret, &offer.run_id) {
                    Ok(claim) => run_claim(&config, claim),
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

fn run_claim(config: &RunnerConfig, claim: ClaimRunResponse) {
    if let Err(error) = execute_claim(config, &claim) {
        eprintln!(
            "Run {} failed before completion: {error:#}",
            claim.job.run_id
        );
        let client = match runner_client() {
            Ok(client) => client,
            Err(client_error) => {
                eprintln!("Could not report failure: {client_error}");
                return;
            }
        };
        let _ = append_attempt_log(
            &client,
            &config.api_url,
            &claim.attempt_token,
            &claim.attempt_id,
            &AppendAttemptLogRequest {
                sequence: 1,
                text: format!("runner error: {error:#}\n"),
            },
        );
        if let Err(report_error) = complete_attempt(
            &client,
            &config.api_url,
            &claim.attempt_token,
            &claim.attempt_id,
            &CompleteAttemptRequest {
                conclusion: AttemptConclusionRequest::Failed { exit_code: 1 },
            },
        ) {
            eprintln!("Could not report failed attempt: {report_error}");
        }
    }
}

fn execute_claim(config: &RunnerConfig, claim: &ClaimRunResponse) -> anyhow::Result<()> {
    let client = runner_client()?;
    let mut supervisor = AttemptSupervisor::start(config.clone(), claim.clone())?;
    let work = RunnerWorkDir::new(&claim.attempt_id)?;
    persist_recovery_claim(&work.path, claim)?;
    let bundle_path = work.path.join("source.bundle");
    let source_client = source_download_client()?;
    download_attempt_source(
        &source_client,
        &config.api_url,
        &claim.attempt_token,
        &claim.attempt_id,
        &claim.job.source_digest,
        &bundle_path,
    )?;
    if finish_before_execution(&mut supervisor, &client, config, claim)? {
        return Ok(());
    }
    let workspace = work.path.join("workspace");
    command_success(
        Command::new("git")
            .args(["clone", "--no-local"])
            .arg(&bundle_path)
            .arg(&workspace),
        "clone run source bundle",
    )?;
    command_success(
        Command::new("git").current_dir(&workspace).args([
            "checkout",
            "--detach",
            &claim.job.git_oid,
        ]),
        "check out exact run commit",
    )?;
    let actual_oid = command_stdout(
        Command::new("git")
            .current_dir(&workspace)
            .args(["rev-parse", "HEAD"]),
        "verify checked-out run commit",
    )?;
    if actual_oid.trim() != claim.job.git_oid {
        bail!("checked-out commit does not match the claimed job");
    }
    let container_image = resolve_container_image(&client, config, claim)?;
    let script_path = work.path.join("job.sh");
    fs::write(&script_path, job_script(&claim.job.workflow)).context("write run script")?;
    if finish_before_execution(&mut supervisor, &client, config, claim)? {
        return Ok(());
    }
    let container_name = format!("scope-{}", claim.attempt_id);
    let mut create = Command::new("docker");
    configure_job_container_creation(
        &mut create,
        config,
        claim,
        &container_name,
        &container_image,
    );
    command_success(&mut create, "create Docker job container")?;
    let container = ContainerGuard::new(container_name);
    command_success(
        Command::new("docker")
            .args(["cp"])
            .arg(format!("{}/.", workspace.display()))
            .arg(format!("{}:/scope-source", container.name)),
        "copy run source into Docker job container",
    )?;
    command_success(
        Command::new("docker")
            .args(["cp"])
            .arg(&script_path)
            .arg(format!("{}:/scope-job.sh", container.name)),
        "copy run script into Docker job container",
    )?;
    if finish_before_execution(&mut supervisor, &client, config, claim)? {
        return Ok(());
    }
    if let Err(error) = attempt_start(
        &client,
        &config.api_url,
        &claim.attempt_token,
        &claim.attempt_id,
    ) {
        thread::sleep(Duration::from_secs(1));
        if finish_before_execution(&mut supervisor, &client, config, claim)? {
            return Ok(());
        }
        return Err(error);
    }
    let mut child = Command::new("docker")
        .args(["start", "--attach", &container.name])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .context("start Docker job container")?;
    let execution_started_at_unix = match container_started_at_unix(&container.name) {
        Ok(started_at) => started_at,
        Err(error) => {
            drop(container);
            let _ = child.wait();
            return Err(error);
        }
    };
    let execution_deadline_unix =
        execution_started_at_unix.saturating_add(claim.job.workflow.timeout_seconds());
    mark_recovery_execution_started(&work.path, claim, execution_deadline_unix)?;
    finish_container_attempt(
        config,
        claim,
        &work.path,
        1,
        0,
        execution_deadline_unix,
        false,
        false,
        &mut supervisor,
        child,
        container,
    )
}

fn resume_claim(config: &RunnerConfig, recovery: RecoveryAttempt) -> anyhow::Result<()> {
    let RecoveryAttempt { work_dir, recovery } = recovery;
    let work = RunnerWorkDir { path: work_dir };
    let claim = recovery.claim;
    let container_name = format!("scope-{}", claim.attempt_id);
    let container = ContainerGuard::new(container_name.clone());
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
    let next_log_sequence = recovery_status.next_log_sequence;
    update_recovery_log_sequence(&work.path, &claim, next_log_sequence)?;
    let execution_deadline_unix = match recovery.progress.execution_deadline_unix {
        Some(deadline) => deadline,
        None => {
            let deadline = container_started_at_unix(&container_name)?
                .saturating_add(claim.job.workflow.timeout_seconds());
            mark_recovery_execution_started(&work.path, &claim, deadline)?;
            deadline
        }
    };
    let mut follow_logs = container_is_running(&container_name)?;
    let deadline_was_elapsed = if follow_logs {
        execution_deadline_unix <= unix_now()
    } else {
        container_finished_at_unix(&container_name)? > execution_deadline_unix
    };
    if deadline_was_elapsed {
        follow_logs = false;
    }
    let mut logs = Command::new("docker");
    logs.arg("logs");
    if follow_logs {
        logs.arg("--follow");
    }
    let child = logs
        .arg(&container_name)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .context("replay interrupted Docker job logs")?;
    let mut supervisor = AttemptSupervisor::start(config.clone(), claim.clone())?;
    finish_container_attempt(
        config,
        &claim,
        &work.path,
        next_log_sequence,
        recovery_status.log_bytes,
        execution_deadline_unix,
        true,
        deadline_was_elapsed,
        &mut supervisor,
        child,
        container,
    )
}

#[allow(clippy::too_many_arguments)]
fn finish_container_attempt(
    config: &RunnerConfig,
    claim: &ClaimRunResponse,
    work_dir: &Path,
    next_log_sequence: u64,
    skip_log_bytes: u64,
    execution_deadline_unix: u64,
    recovering: bool,
    recovered_deadline_elapsed: bool,
    supervisor: &mut AttemptSupervisor,
    mut child: Child,
    container: ContainerGuard,
) -> anyhow::Result<()> {
    if let Err(error) = supervisor.set_container(container.name.clone()) {
        drop(container);
        let _ = child.wait();
        return Err(error);
    }
    supervisor.set_execution_deadline(if recovered_deadline_elapsed {
        u64::MAX
    } else {
        execution_deadline_unix
    });
    let status = stream_logs(
        config,
        claim,
        work_dir,
        next_log_sequence,
        skip_log_bytes,
        &mut child,
    );
    if status.is_err() {
        let _ = child.wait();
    }
    let status = status?;
    let stop_reason = supervisor.finish();
    let exit_code = if recovered_deadline_elapsed {
        None
    } else if stop_reason == AttemptStopReason::None && recovering {
        recovered_container_exit_code(&container.name)?
    } else {
        status.code()
    };
    drop(container);

    let conclusion = match (recovered_deadline_elapsed, stop_reason) {
        (true, _) => AttemptConclusionRequest::Failed { exit_code: 124 },
        (false, AttemptStopReason::Cancellation) => AttemptConclusionRequest::Canceled,
        (false, AttemptStopReason::TimedOut) => AttemptConclusionRequest::Failed { exit_code: 124 },
        (false, AttemptStopReason::LeaseLost) => AttemptConclusionRequest::Failed { exit_code: 70 },
        (false, AttemptStopReason::None) if exit_code == Some(0) => {
            AttemptConclusionRequest::Succeeded
        }
        (false, AttemptStopReason::None) => AttemptConclusionRequest::Failed {
            exit_code: exit_code.unwrap_or(1).max(1),
        },
    };
    let client = runner_client()?;
    complete_attempt(
        &client,
        &config.api_url,
        &claim.attempt_token,
        &claim.attempt_id,
        &CompleteAttemptRequest { conclusion },
    )?;
    Ok(())
}

fn download_attempt_source(
    client: &Client,
    api_url: &str,
    attempt_token: &str,
    attempt_id: &str,
    expected_digest: &str,
    destination: &Path,
) -> anyhow::Result<()> {
    let mut response = attempt_source(client, api_url, attempt_token, attempt_id)?;
    if response
        .content_length()
        .is_some_and(|length| length > MAX_SOURCE_BUNDLE_BYTES)
    {
        bail!("run source bundle exceeds {MAX_SOURCE_BUNDLE_BYTES} bytes");
    }
    let mut file = fs::OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(destination)
        .context("create run source bundle")?;
    let mut hasher = Sha256::new();
    let mut total = 0_u64;
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let read = response
            .read(&mut buffer)
            .context("stream run source bundle")?;
        if read == 0 {
            break;
        }
        total = total
            .checked_add(read as u64)
            .context("run source bundle byte count overflow")?;
        if total > MAX_SOURCE_BUNDLE_BYTES {
            bail!("run source bundle exceeds {MAX_SOURCE_BUNDLE_BYTES} bytes");
        }
        file.write_all(&buffer[..read])
            .context("write run source bundle")?;
        hasher.update(&buffer[..read]);
    }
    file.sync_all().context("sync run source bundle")?;
    let actual_digest = format!("{:x}", hasher.finalize());
    if actual_digest != expected_digest {
        bail!("downloaded source digest does not match claimed job");
    }
    Ok(())
}

fn finish_before_execution(
    supervisor: &mut AttemptSupervisor,
    client: &Client,
    config: &RunnerConfig,
    claim: &ClaimRunResponse,
) -> anyhow::Result<bool> {
    match supervisor.reason() {
        AttemptStopReason::None => Ok(false),
        AttemptStopReason::Cancellation => {
            let _ = supervisor.finish();
            complete_canceled(client, config, claim)?;
            Ok(true)
        }
        AttemptStopReason::LeaseLost => {
            let _ = supervisor.finish();
            complete_attempt(
                client,
                &config.api_url,
                &claim.attempt_token,
                &claim.attempt_id,
                &CompleteAttemptRequest {
                    conclusion: AttemptConclusionRequest::Failed { exit_code: 70 },
                },
            )?;
            Ok(true)
        }
        AttemptStopReason::TimedOut => {
            let _ = supervisor.finish();
            bail!("attempt timed out before execution started")
        }
    }
}

fn stream_logs(
    config: &RunnerConfig,
    claim: &ClaimRunResponse,
    work_dir: &Path,
    mut next_sequence: u64,
    mut skip_bytes: u64,
    child: &mut Child,
) -> anyhow::Result<std::process::ExitStatus> {
    let stdout = child.stdout.take().context("Docker stdout was not piped")?;
    let stderr = child.stderr.take().context("Docker stderr was not piped")?;
    let (sender, receiver) = mpsc::channel();
    let stdout_thread = spawn_log_reader(stdout, sender);
    let stderr_thread = spawn_diagnostic_reader(stderr);
    let client = attempt_control_client()?;
    let mut upload_logs = true;
    for mut text in receiver {
        if skip_bytes != 0 {
            let text_bytes = text.as_bytes();
            if skip_bytes >= text_bytes.len() as u64 {
                skip_bytes -= text_bytes.len() as u64;
                continue;
            }
            let start = usize::try_from(skip_bytes).context("recovery log cursor is too large")?;
            text = String::from_utf8_lossy(&text_bytes[start..]).into_owned();
            skip_bytes = 0;
        }
        print!("{text}");
        let _ = std::io::stdout().flush();
        if upload_logs {
            match append_log_with_retry(&client, config, claim, next_sequence, text) {
                Ok(true) => {
                    next_sequence = next_sequence
                        .checked_add(1)
                        .context("run log sequence overflow")?;
                    update_recovery_log_sequence(work_dir, claim, next_sequence)?;
                }
                Ok(false) => {
                    upload_logs = false;
                    eprintln!(
                        "\nScope stopped uploading logs after the per-attempt log limit was reached."
                    );
                }
                Err(error) => {
                    upload_logs = false;
                    eprintln!(
                        "\nScope log upload failed; the job will continue with local-only output: {error:#}"
                    );
                }
            }
        }
    }
    let _ = stdout_thread.join();
    let _ = stderr_thread.join();
    if skip_bytes != 0 {
        bail!("recovered Docker logs are shorter than the server log cursor");
    }
    child.wait().context("wait for Docker job container")
}

fn spawn_log_reader(
    stream: impl Read + Send + 'static,
    sender: mpsc::Sender<String>,
) -> thread::JoinHandle<()> {
    thread::spawn(move || {
        let mut reader = BufReader::new(stream);
        let mut bytes = vec![0_u8; LOG_CHUNK_BYTES];
        loop {
            match reader.read(&mut bytes) {
                Ok(0) => break,
                Ok(read) => {
                    if sender.send(stable_log_text(&bytes[..read])).is_err() {
                        break;
                    }
                }
                Err(_) => break,
            }
        }
    })
}

fn stable_log_text(bytes: &[u8]) -> String {
    let mut text = String::with_capacity(bytes.len());
    for &byte in bytes {
        match byte {
            b'\n' | b'\r' | b'\t' | 0x20..=0x7e => text.push(char::from(byte)),
            _ => {
                use std::fmt::Write as _;
                write!(text, "\\x{byte:02x}").expect("writing to a String cannot fail");
            }
        }
    }
    text
}

fn spawn_diagnostic_reader(stream: impl Read + Send + 'static) -> thread::JoinHandle<()> {
    thread::spawn(move || {
        let mut reader = BufReader::new(stream);
        let mut bytes = vec![0_u8; LOG_CHUNK_BYTES];
        loop {
            match reader.read(&mut bytes) {
                Ok(0) => break,
                Ok(read) => {
                    eprint!("{}", String::from_utf8_lossy(&bytes[..read]));
                    let _ = std::io::stderr().flush();
                }
                Err(_) => break,
            }
        }
    })
}

fn append_log_with_retry(
    client: &Client,
    config: &RunnerConfig,
    claim: &ClaimRunResponse,
    sequence: u64,
    text: String,
) -> anyhow::Result<bool> {
    let request = AppendAttemptLogRequest { sequence, text };
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

fn job_script(workflow: &scope_domain::runs::workflow::CompiledWorkflow) -> String {
    let mut script = String::from("#!/bin/sh\nset -e\n");
    for step in workflow.steps() {
        script.push_str("printf '\\n==> %s\\n' ");
        script.push_str(&shell_quote(step.name()));
        script.push('\n');
        script.push_str(step.run());
        if !step.run().ends_with('\n') {
            script.push('\n');
        }
        script.push_str("printf '<== %s\\n' ");
        script.push_str(&shell_quote(step.name()));
        script.push('\n');
    }
    script
}

fn shell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\"'\"'"))
}

fn print_runner_status(name: &str, runner: &RunnerResponse) {
    let online = runner
        .last_seen_at_unix
        .and_then(|last_seen| unix_now().checked_sub(last_seen))
        .is_some_and(|age| age <= 90);
    println!(
        "{} · {} · {}",
        name,
        if online { "online" } else { "offline" },
        if runner.enabled {
            "enabled"
        } else {
            "disabled"
        }
    );
    for grant in runner.grants.iter().filter(|grant| grant.active) {
        println!("  {} as {}", grant.repository_id, grant.name);
    }
}

fn parse_repository(repository: &str) -> anyhow::Result<(&str, &str)> {
    let mut parts = repository.split('/');
    let owner = parts.next().unwrap_or_default();
    let repo = parts.next().unwrap_or_default();
    if owner.is_empty() || repo.is_empty() || parts.next().is_some() {
        bail!("expected repository as owner/repo");
    }
    Ok((owner, repo))
}

fn runner_client() -> anyhow::Result<Client> {
    Client::builder()
        .timeout(Duration::from_secs(35))
        .build()
        .context("build runner HTTP client")
}

fn source_download_client() -> anyhow::Result<Client> {
    Client::builder()
        .connect_timeout(Duration::from_secs(20))
        .timeout(Duration::from_secs(15 * 60))
        .build()
        .context("build run source download client")
}

fn attempt_control_client() -> anyhow::Result<Client> {
    Client::builder()
        .timeout(Duration::from_secs(10))
        .build()
        .context("build attempt control HTTP client")
}

fn runner_config_path() -> anyhow::Result<PathBuf> {
    if let Some(path) = env::var_os("SCOPE_RUNNER_CONFIG").filter(|path| !path.is_empty()) {
        return Ok(PathBuf::from(path));
    }
    Ok(scope_config_home()?.join("scope/runner.json"))
}

fn scope_config_home() -> anyhow::Result<PathBuf> {
    env::var_os("XDG_CONFIG_HOME")
        .filter(|path| !path.is_empty())
        .map(PathBuf::from)
        .or_else(|| {
            env::var_os("HOME")
                .filter(|path| !path.is_empty())
                .map(|home| PathBuf::from(home).join(".config"))
        })
        .context("XDG_CONFIG_HOME or HOME is required")
}

fn store_runner_config(path: &Path, config: &RunnerConfig) -> anyhow::Result<()> {
    let parent = path
        .parent()
        .context("runner config path has no parent directory")?;
    fs::create_dir_all(parent).context("create runner config directory")?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
        let mut permissions = fs::metadata(parent)?.permissions();
        permissions.set_mode(0o700);
        fs::set_permissions(parent, permissions)?;
        let temp = path.with_extension(format!("tmp.{}", std::process::id()));
        let mut file = fs::OpenOptions::new()
            .create(true)
            .truncate(true)
            .write(true)
            .mode(0o600)
            .open(&temp)
            .context("create runner config")?;
        serde_json::to_writer(&mut file, config).context("serialize runner config")?;
        file.write_all(b"\n")?;
        file.sync_all()?;
        fs::rename(temp, path).context("install runner config")?;
    }
    #[cfg(not(unix))]
    {
        fs::write(path, serde_json::to_vec(config)?).context("write runner config")?;
    }
    Ok(())
}

fn load_runner_config() -> anyhow::Result<RunnerConfig> {
    let path = runner_config_path()?;
    load_runner_config_from(&path)
}

fn load_runner_config_from(path: &Path) -> anyhow::Result<RunnerConfig> {
    let bytes = fs::read(path).with_context(|| {
        format!(
            "read runner config {}; run scope runner install",
            path.display()
        )
    })?;
    serde_json::from_slice(&bytes).context("parse runner config")
}

fn command_success(command: &mut Command, context: &str) -> anyhow::Result<()> {
    let output = command.output().with_context(|| context.to_string())?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        bail!("{context}: {}", stderr.trim());
    }
    Ok(())
}

fn command_stdout(command: &mut Command, context: &str) -> anyhow::Result<String> {
    let output = command.output().with_context(|| context.to_string())?;
    if !output.status.success() {
        bail!(
            "{context}: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

fn unix_now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |duration| duration.as_secs())
}

struct RunnerWorkDir {
    path: PathBuf,
}

impl RunnerWorkDir {
    fn new(attempt_id: &str) -> anyhow::Result<Self> {
        let base = runner_work_root()?;
        fs::create_dir_all(&base).context("create runner work directory")?;
        let metadata = fs::symlink_metadata(&base)?;
        if !metadata.file_type().is_dir() {
            bail!("runner work path is not a directory");
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut permissions = metadata.permissions();
            permissions.set_mode(0o700);
            fs::set_permissions(&base, permissions)?;
        }
        let safe_id = attempt_id
            .chars()
            .map(|character| {
                if character.is_ascii_alphanumeric() || character == '_' || character == '-' {
                    character
                } else {
                    '_'
                }
            })
            .collect::<String>();
        let path = base.join(safe_id);
        fs::create_dir(&path).context("create attempt work directory")?;
        Ok(Self { path })
    }
}

fn runner_work_root() -> anyhow::Result<PathBuf> {
    Ok(runner_state_home()?.join("scope/runner-work"))
}

fn runner_state_home() -> anyhow::Result<PathBuf> {
    env::var_os("XDG_STATE_HOME")
        .filter(|path| !path.is_empty())
        .map(PathBuf::from)
        .or_else(|| {
            env::var_os("HOME")
                .filter(|path| !path.is_empty())
                .map(|home| PathBuf::from(home).join(".local/state"))
        })
        .context("XDG_STATE_HOME or HOME is required")
}

impl Drop for RunnerWorkDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

#[cfg(test)]
mod tests;
