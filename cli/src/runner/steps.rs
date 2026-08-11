use super::{
    ConclusionReportPending, ExecutionOutcome, RunnerConfig, RunnerWorkDir,
    conclude_stopped_attempt,
    container::{ContainerGuard, stop_container},
    recovery::{
        PendingLogChunk, mark_recovery_abandon_pending, mark_recovery_conclusion_pending,
        mark_recovery_step_completed, mark_recovery_step_conclusion_pending,
        mark_recovery_step_started, update_recovery_log_progress,
    },
    step_logs::{copy_step_log, drain_step_logs, step_log_was_truncated},
    supervisor::{AttemptStopReason, AttemptSupervisor},
};
use crate::api::{attempt_recovery_status, complete_attempt_step, start_attempt_step};
use anyhow::{Context, bail};
use reqwest::blocking::Client;
use scope_api_contract::{ClaimRunResponse, CompleteAttemptStepRequest, StepConclusionRequest};
use scope_domain::runs::run::StepState;
use scope_domain::runs::workflow::WorkflowJob;
use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
    thread,
    time::{Duration, Instant},
};

const CONTAINER_ACTIVE_STEP: &str = "/scope-active-step";

pub(super) fn write_step_programs(work_dir: &Path, job: &WorkflowJob) -> anyhow::Result<PathBuf> {
    let programs = work_dir.join("steps");
    fs::create_dir(&programs).context("create runner step program directory")?;
    for (index, step) in job.steps().iter().enumerate() {
        let path = programs.join(format!("step-{index}.sh"));
        fs::write(&path, step.run()).with_context(|| format!("write workflow step {index}"))?;
    }
    Ok(programs)
}

fn select_container_step(
    programs: &Path,
    phase: &str,
    step_index: u32,
    nonce: &str,
) -> anyhow::Result<()> {
    if !matches!(phase, "prepare" | "run") {
        bail!("workflow step phase is invalid");
    }
    let path = programs.join("current");
    let temporary = programs.join(".current.tmp");
    fs::write(&temporary, format!("{phase} {step_index} {nonce}\n"))
        .context("write current workflow step")?;
    fs::rename(&temporary, &path).context("publish current workflow step")?;
    fs::File::open(programs)
        .context("open workflow step directory")?
        .sync_all()
        .context("persist current workflow step directory")
}

pub(super) fn step_exit_code(container_name: &str) -> anyhow::Result<Option<i32>> {
    let output = Command::new("docker")
        .args([
            "container",
            "inspect",
            "--format={{.State.Running}} {{.State.ExitCode}}",
            container_name,
        ])
        .output()
        .context("inspect workflow step result")?;
    if !output.status.success() {
        bail!(
            "inspect workflow step result: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    let value = String::from_utf8_lossy(&output.stdout);
    let (running, exit_code) = value
        .trim()
        .split_once(' ')
        .context("workflow step container state is incomplete")?;
    match running {
        "true" => Ok(None),
        "false" => Ok(Some(
            exit_code
                .parse::<i32>()
                .context("parse workflow step exit code")?,
        )),
        _ => bail!("workflow step container running state is invalid"),
    }
}

fn container_step_nonce(
    container_name: &str,
    destination: &Path,
) -> anyhow::Result<Option<String>> {
    let output = Command::new("docker")
        .args(["cp", &format!("{container_name}:{CONTAINER_ACTIVE_STEP}")])
        .arg(destination)
        .output()
        .context("inspect active workflow step")?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let normalized = stderr.to_ascii_lowercase();
        if normalized.contains("could not find") || normalized.contains("no such container") {
            return Ok(None);
        }
        bail!("inspect active workflow step: {}", stderr.trim());
    }
    Ok(Some(
        fs::read_to_string(destination)
            .context("read active workflow step")?
            .trim()
            .to_string(),
    ))
}

fn new_step_nonce() -> anyhow::Result<String> {
    let mut bytes = [0_u8; 16];
    getrandom::fill(&mut bytes)
        .map_err(|error| anyhow::anyhow!("generate workflow step nonce: {error}"))?;
    Ok(hex::encode(bytes))
}

#[allow(clippy::too_many_arguments)]
pub(super) fn run_steps(
    config: &RunnerConfig,
    claim: &ClaimRunResponse,
    work: &mut RunnerWorkDir,
    client: &Client,
    first_step_index: u32,
    mut next_log_sequence: u64,
    mut step_log_bytes: u64,
    mut logs_exhausted: bool,
    mut pending_log_chunk: Option<PendingLogChunk>,
    mut active_step_nonce: Option<String>,
    execution_deadline_unix: u64,
    supervisor: &mut AttemptSupervisor,
    mut container: ContainerGuard,
) -> anyhow::Result<ExecutionOutcome> {
    supervisor.set_execution_deadline(execution_deadline_unix);
    let step_count = u32::try_from(super::dispatch_job(claim)?.steps().len())
        .context("workflow step count exceeds runner protocol")?;
    let programs = work.path.join("steps");
    for step_index in first_step_index..step_count {
        let recovering_step = active_step_nonce.is_some();
        let reason = supervisor.reason();
        if reason != AttemptStopReason::None && !recovering_step {
            stage_stop_reason_or_preserve(work, claim, reason, &mut container)?;
            stop_attempt_container(work, &mut container)?;
            return conclude_stopped_attempt(config, claim, work, reason, logs_exhausted)
                .map(|()| ExecutionOutcome::Interrupted);
        }
        let nonce = match active_step_nonce.take() {
            Some(nonce) => nonce,
            None => {
                let nonce = new_step_nonce()?;
                mark_recovery_step_started(&work.path, claim, step_index, &nonce)?;
                nonce
            }
        };
        let snapshot = work.path.join(format!("step-{step_index}.log"));
        let nonce_snapshot = work.path.join("container-active-step");
        let observed_nonce_result = container_step_nonce(&container.name, &nonce_snapshot);
        let observed_nonce = preserve_operation_if_needed(
            observed_nonce_result,
            recovering_step,
            work,
            claim,
            &mut container,
            &snapshot,
            "recovered container inspection",
        )?;
        let mut observed_exit = if observed_nonce.as_deref() == Some(&nonce) {
            let exit_result = step_exit_code(&container.name);
            preserve_operation_if_needed(
                exit_result,
                recovering_step,
                work,
                claim,
                &mut container,
                &snapshot,
                "recovered container inspection",
            )?
        } else {
            let exit_result = step_exit_code(&container.name);
            let exit = preserve_operation_if_needed(
                exit_result,
                recovering_step,
                work,
                claim,
                &mut container,
                &snapshot,
                "recovered container inspection",
            )?;
            if exit.is_none() && recovering_step {
                None
            } else if exit.is_none() {
                let error =
                    anyhow::anyhow!("running attempt container does not match its recovery step");
                return Err(error);
            } else {
                let prepare_result =
                    select_container_step(&programs, "prepare", step_index, &nonce);
                preserve_operation_if_needed(
                    prepare_result,
                    recovering_step,
                    work,
                    claim,
                    &mut container,
                    &snapshot,
                    "recovered step preparation",
                )?;
                let started = Instant::now();
                let start_result = command_success(
                    Command::new("docker").args(["start", &container.name]),
                    "start workflow step",
                );
                eprintln!(
                    "scope_runner_phase attempt_id={} phase=container_start step={} elapsed_ms={}",
                    claim.attempt_id,
                    step_index,
                    started.elapsed().as_millis()
                );
                preserve_operation_if_needed(
                    start_result,
                    recovering_step,
                    work,
                    claim,
                    &mut container,
                    &snapshot,
                    "recovered step start",
                )?;
                None
            }
        };
        if observed_exit
            .is_some_and(|exit_code| exit_code != 0 || step_index.saturating_add(1) == step_count)
        {
            supervisor.mark_execution_finished();
        }
        loop {
            let nonce_result = container_step_nonce(&container.name, &nonce_snapshot);
            let observed_nonce = preserve_operation_if_needed(
                nonce_result,
                recovering_step,
                work,
                claim,
                &mut container,
                &snapshot,
                "recovered step preparation",
            )?;
            if observed_nonce.as_deref() == Some(&nonce) {
                break;
            }
            let exit_result = step_exit_code(&container.name);
            let inspected_exit = preserve_operation_if_needed(
                exit_result,
                recovering_step,
                work,
                claim,
                &mut container,
                &snapshot,
                "recovered step preparation",
            )?;
            if let Some(exit_code) = observed_exit.take().or(inspected_exit) {
                let error = anyhow::anyhow!(
                    "workflow step container setup exited with code {exit_code} before execution"
                );
                if recovering_step {
                    return preserve_after_runner_failure(
                        work,
                        claim,
                        &mut container,
                        &snapshot,
                        "recovered step preparation",
                        error,
                    );
                }
                return Err(error);
            }
            match supervisor.reason() {
                AttemptStopReason::None => thread::sleep(Duration::from_millis(50)),
                reason => {
                    stage_stop_reason_or_preserve(work, claim, reason, &mut container)?;
                    stop_attempt_container(work, &mut container)?;
                    return conclude_stopped_attempt(config, claim, work, reason, logs_exhausted)
                        .map(|()| ExecutionOutcome::Interrupted);
                }
            }
        }
        if let Some(reason) = ensure_step_started(client, config, claim, step_index, supervisor)? {
            stage_stop_reason_or_preserve(work, claim, reason, &mut container)?;
            stop_attempt_container(work, &mut container)?;
            let drain_result = drain_step_logs(
                client,
                config,
                claim,
                &work.path,
                &container.name,
                step_index,
                &snapshot,
                &mut next_log_sequence,
                &mut step_log_bytes,
                &mut logs_exhausted,
                &mut pending_log_chunk,
                true,
            );
            preserve_stopped_log_recovery(work, &mut container, drain_result)?;
            return conclude_stopped_attempt(config, claim, work, reason, logs_exhausted)
                .map(|()| ExecutionOutcome::Interrupted);
        }
        if let Err(error) = select_container_step(&programs, "run", step_index, &nonce) {
            return preserve_after_runner_failure(
                work,
                claim,
                &mut container,
                &snapshot,
                "step release",
                error,
            );
        }
        let exit_code = loop {
            if observed_exit.is_none() {
                observed_exit = match step_exit_code(&container.name) {
                    Ok(exit_code) => exit_code,
                    Err(error) => {
                        return preserve_after_runner_failure(
                            work,
                            claim,
                            &mut container,
                            &snapshot,
                            "container inspection",
                            error,
                        );
                    }
                };
            }
            if observed_exit.is_some_and(|exit_code| {
                exit_code != 0 || step_index.saturating_add(1) == step_count
            }) {
                supervisor.mark_execution_finished();
            }
            match supervisor.reason() {
                AttemptStopReason::None => {}
                reason => {
                    stage_stop_reason_or_preserve(work, claim, reason, &mut container)?;
                    stop_attempt_container(work, &mut container)?;
                    let drain_result = drain_step_logs(
                        client,
                        config,
                        claim,
                        &work.path,
                        &container.name,
                        step_index,
                        &snapshot,
                        &mut next_log_sequence,
                        &mut step_log_bytes,
                        &mut logs_exhausted,
                        &mut pending_log_chunk,
                        true,
                    );
                    preserve_stopped_log_recovery(work, &mut container, drain_result)?;
                    return conclude_stopped_attempt(config, claim, work, reason, logs_exhausted)
                        .map(|()| ExecutionOutcome::Interrupted);
                }
            }
            let drain_result = drain_step_logs(
                client,
                config,
                claim,
                &work.path,
                &container.name,
                step_index,
                &snapshot,
                &mut next_log_sequence,
                &mut step_log_bytes,
                &mut logs_exhausted,
                &mut pending_log_chunk,
                false,
            );
            if let Err(error) = drain_result {
                let reason = supervisor.reason();
                if reason != AttemptStopReason::None {
                    stage_stop_reason_or_preserve(work, claim, reason, &mut container)?;
                    stop_attempt_container(work, &mut container)?;
                    let final_drain_result = drain_step_logs(
                        client,
                        config,
                        claim,
                        &work.path,
                        &container.name,
                        step_index,
                        &snapshot,
                        &mut next_log_sequence,
                        &mut step_log_bytes,
                        &mut logs_exhausted,
                        &mut pending_log_chunk,
                        true,
                    );
                    preserve_stopped_log_recovery(work, &mut container, final_drain_result)?;
                    return conclude_stopped_attempt(config, claim, work, reason, logs_exhausted)
                        .map(|()| ExecutionOutcome::Interrupted);
                }
                return preserve_after_runner_failure(
                    work,
                    claim,
                    &mut container,
                    &snapshot,
                    "log handling",
                    error,
                );
            }
            if observed_exit.is_none() {
                observed_exit = match step_exit_code(&container.name) {
                    Ok(exit_code) => exit_code,
                    Err(error) => {
                        return preserve_after_runner_failure(
                            work,
                            claim,
                            &mut container,
                            &snapshot,
                            "container inspection",
                            error,
                        );
                    }
                };
            }
            if observed_exit.is_some_and(|exit_code| {
                exit_code != 0 || step_index.saturating_add(1) == step_count
            }) {
                supervisor.mark_execution_finished();
            }
            if supervisor.reason() != AttemptStopReason::None {
                continue;
            }
            if let Some(exit_code) = observed_exit.take() {
                break exit_code;
            }
            thread::sleep(Duration::from_millis(500));
        };
        let storage_pressure = supervisor.storage_pressure_triggered();
        let final_drain_result = if storage_pressure {
            match step_log_was_truncated(&container.name, &work.path) {
                Ok(true) => {
                    logs_exhausted = true;
                    update_recovery_log_progress(
                        &work.path,
                        claim,
                        step_index,
                        next_log_sequence,
                        step_log_bytes,
                        true,
                    )
                }
                Ok(false) => Ok(()),
                Err(error) => Err(error),
            }
        } else {
            drain_step_logs(
                client,
                config,
                claim,
                &work.path,
                &container.name,
                step_index,
                &snapshot,
                &mut next_log_sequence,
                &mut step_log_bytes,
                &mut logs_exhausted,
                &mut pending_log_chunk,
                true,
            )
        };
        if let Err(error) = final_drain_result {
            work.preserve();
            container.preserve();
            eprintln!(
                "Could not finish the completed workflow step log upload; recovery will retry: {error:#}"
            );
            return Err(ConclusionReportPending.into());
        }
        if storage_pressure {
            eprintln!(
                "Attempt {} step {} was stopped after runner storage crossed its emergency floor",
                claim.attempt_id, step_index
            );
        }
        let exit_code = exit_code_after_storage_guard(exit_code, storage_pressure);
        let conclusion = if exit_code == 0 {
            StepConclusionRequest::Succeeded
        } else {
            StepConclusionRequest::Failed { exit_code }
        };
        if let Err(error) = mark_recovery_step_conclusion_pending(
            &work.path,
            claim,
            step_index,
            conclusion.clone(),
            logs_exhausted,
        ) {
            work.preserve();
            container.preserve();
            eprintln!(
                "Could not persist the completed workflow step; recovery will retry: {error:#}"
            );
            return Err(ConclusionReportPending.into());
        }
        if let Err(error) = report_step_conclusion_until_reconciled(
            client,
            config,
            claim,
            work,
            step_index,
            conclusion,
            logs_exhausted,
            supervisor,
        ) {
            container.preserve();
            return Err(error);
        }
        if let Err(error) = mark_recovery_step_completed(&work.path, claim, step_index) {
            work.preserve();
            container.preserve();
            eprintln!("Could not commit reconciled step recovery progress: {error:#}");
            return Err(ConclusionReportPending.into());
        }
        if exit_code != 0 {
            supervisor.mark_execution_finished();
            return Ok(if storage_pressure {
                ExecutionOutcome::Interrupted
            } else {
                ExecutionOutcome::Failed
            });
        }
        step_log_bytes = 0;
    }
    supervisor.mark_execution_finished();
    Ok(ExecutionOutcome::Succeeded)
}

fn exit_code_after_storage_guard(observed: i32, storage_pressure: bool) -> i32 {
    if storage_pressure { 1 } else { observed }
}

pub(super) fn report_step_conclusion(
    client: &Client,
    config: &RunnerConfig,
    claim: &ClaimRunResponse,
    step_index: u32,
    conclusion: StepConclusionRequest,
    logs_truncated: bool,
) -> anyhow::Result<()> {
    let request = CompleteAttemptStepRequest {
        conclusion: conclusion.clone(),
        logs_truncated,
    };
    let mut last_error = None;
    for _ in 0..3 {
        match complete_attempt_step(
            client,
            &config.api_url,
            &claim.attempt_token,
            &claim.attempt_id,
            step_index,
            &request,
        ) {
            Ok(_) => return Ok(()),
            Err(error) => last_error = Some(error),
        }
        if server_step_matches(client, config, claim, step_index, &conclusion)? {
            return Ok(());
        }
        thread::sleep(Duration::from_secs(1));
    }
    Err(last_error.expect("step conclusion retry records an error"))
}

fn ensure_step_started(
    client: &Client,
    config: &RunnerConfig,
    claim: &ClaimRunResponse,
    step_index: u32,
    supervisor: &AttemptSupervisor,
) -> anyhow::Result<Option<AttemptStopReason>> {
    loop {
        let reason = supervisor.reason();
        if reason != AttemptStopReason::None {
            return Ok(Some(reason));
        }
        match start_attempt_step(
            client,
            &config.api_url,
            &claim.attempt_token,
            &claim.attempt_id,
            step_index,
        ) {
            Ok(_) => return Ok(None),
            Err(error) => {
                if server_step_is_running(client, config, claim, step_index).unwrap_or(false) {
                    return Ok(None);
                }
                eprintln!("Could not confirm workflow step start; retrying: {error:#}");
                thread::sleep(Duration::from_secs(1));
            }
        }
    }
}

#[allow(clippy::too_many_arguments)]
pub(super) fn report_step_conclusion_until_reconciled(
    client: &Client,
    config: &RunnerConfig,
    claim: &ClaimRunResponse,
    work: &mut RunnerWorkDir,
    step_index: u32,
    conclusion: StepConclusionRequest,
    logs_truncated: bool,
    supervisor: &AttemptSupervisor,
) -> anyhow::Result<()> {
    for _ in 0..3 {
        match report_step_conclusion(
            client,
            config,
            claim,
            step_index,
            conclusion.clone(),
            logs_truncated,
        ) {
            Ok(()) => return Ok(()),
            Err(error) => {
                let reason = supervisor.reason();
                if reason != AttemptStopReason::None {
                    let final_step = step_index.saturating_add(1)
                        == u32::try_from(super::dispatch_job(claim)?.steps().len())
                            .context("workflow step count exceeds runner protocol")?;
                    let terminal_step = matches!(&conclusion, StepConclusionRequest::Failed { .. })
                        || matches!(&conclusion, StepConclusionRequest::Succeeded) && final_step;
                    if terminal_step {
                        work.preserve();
                        eprintln!(
                            "Could not report the terminal workflow step before {reason:?}: {error:#}"
                        );
                        return Err(ConclusionReportPending.into());
                    }
                    if let Err(persist_error) = stage_stop_reason(&work.path, claim, reason) {
                        work.preserve();
                        eprintln!(
                            "Could not persist the stop reason while reconciling a step: {persist_error:#}"
                        );
                        return Err(ConclusionReportPending.into());
                    }
                    work.preserve();
                    eprintln!(
                        "Could not report the persisted workflow step conclusion before {reason:?}: {error:#}"
                    );
                    return Err(ConclusionReportPending.into());
                }
                eprintln!("Could not confirm workflow step conclusion; retrying: {error:#}");
                thread::sleep(Duration::from_secs(1));
            }
        }
    }
    work.preserve();
    eprintln!("Workflow step conclusion remains persisted for runner recovery");
    Err(ConclusionReportPending.into())
}

fn server_step_is_running(
    client: &Client,
    config: &RunnerConfig,
    claim: &ClaimRunResponse,
    step_index: u32,
) -> anyhow::Result<bool> {
    Ok(attempt_recovery_status(
        client,
        &config.api_url,
        &claim.attempt_token,
        &claim.attempt_id,
    )?
    .steps
    .iter()
    .any(|step| step.step_index == step_index && step.state == StepState::Running))
}

fn server_step_matches(
    client: &Client,
    config: &RunnerConfig,
    claim: &ClaimRunResponse,
    step_index: u32,
    conclusion: &StepConclusionRequest,
) -> anyhow::Result<bool> {
    let status = attempt_recovery_status(
        client,
        &config.api_url,
        &claim.attempt_token,
        &claim.attempt_id,
    )?;
    Ok(status.steps.iter().any(|step| {
        if step.step_index != step_index {
            return false;
        }
        match conclusion {
            StepConclusionRequest::Succeeded => {
                step.state == StepState::Succeeded && step.exit_code == Some(0)
            }
            StepConclusionRequest::Failed { exit_code } => {
                step.state == StepState::Failed && step.exit_code == Some(*exit_code)
            }
        }
    }))
}

fn stage_stop_reason(
    work_dir: &Path,
    claim: &ClaimRunResponse,
    reason: AttemptStopReason,
) -> anyhow::Result<()> {
    let conclusion = match reason {
        AttemptStopReason::Cancellation => {
            Some(scope_api_contract::AttemptConclusionRequest::Canceled)
        }
        AttemptStopReason::TimedOut => Some(scope_api_contract::AttemptConclusionRequest::TimedOut),
        AttemptStopReason::LeaseLost => {
            mark_recovery_abandon_pending(work_dir, claim)?;
            None
        }
        AttemptStopReason::None => None,
    };
    if let Some(conclusion) = conclusion {
        mark_recovery_conclusion_pending(work_dir, claim, conclusion)?;
    }
    Ok(())
}

fn stage_stop_reason_or_preserve(
    work: &mut RunnerWorkDir,
    claim: &ClaimRunResponse,
    reason: AttemptStopReason,
    container: &mut ContainerGuard,
) -> anyhow::Result<()> {
    stage_stop_reason(&work.path, claim, reason).map_err(|error| {
        work.preserve();
        container.preserve();
        eprintln!("Could not persist the attempt stop reason; recovery will retry: {error:#}");
        anyhow::Error::from(ConclusionReportPending)
    })
}

fn stop_attempt_container(
    work: &mut RunnerWorkDir,
    container: &mut ContainerGuard,
) -> anyhow::Result<()> {
    stop_container(&container.name).map_err(|error| {
        work.preserve();
        container.preserve();
        eprintln!("Could not confirm stopped attempt execution ended: {error:#}");
        anyhow::Error::from(ConclusionReportPending)
    })
}

fn preserve_stopped_log_recovery(
    work: &mut RunnerWorkDir,
    container: &mut ContainerGuard,
    result: anyhow::Result<()>,
) -> anyhow::Result<()> {
    result.map_err(|error| {
        work.preserve();
        container.preserve();
        eprintln!("Could not finish stopped-step log upload: {error:#}");
        anyhow::Error::from(ConclusionReportPending)
    })
}

fn preserve_operation_if_needed<T>(
    result: anyhow::Result<T>,
    preserve: bool,
    work: &mut RunnerWorkDir,
    claim: &ClaimRunResponse,
    container: &mut ContainerGuard,
    snapshot: &Path,
    operation: &str,
) -> anyhow::Result<T> {
    match result {
        Ok(value) => Ok(value),
        Err(error) if preserve => {
            preserve_after_runner_failure(work, claim, container, snapshot, operation, error)
        }
        Err(error) => Err(error),
    }
}

fn preserve_after_runner_failure<T>(
    work: &mut RunnerWorkDir,
    claim: &ClaimRunResponse,
    container: &mut ContainerGuard,
    snapshot: &Path,
    operation: &str,
    error: anyhow::Error,
) -> anyhow::Result<T> {
    if let Err(recovery_error) = mark_recovery_abandon_pending(&work.path, claim) {
        work.preserve();
        container.preserve();
        eprintln!(
            "Could not persist runner-loss recovery after log handling failed: {recovery_error:#}"
        );
        return Err(ConclusionReportPending.into());
    }
    stop_attempt_container(work, container)?;
    if let Err(snapshot_error) = copy_step_log(&container.name, snapshot) {
        eprintln!("Could not preserve the final stopped-step log snapshot: {snapshot_error:#}");
    }
    work.preserve();
    container.preserve();
    eprintln!(
        "Workflow execution stopped after {operation} failed; recovery will retry: {error:#}"
    );
    Err(ConclusionReportPending.into())
}

fn command_success(command: &mut Command, context: &str) -> anyhow::Result<()> {
    let output = command.output().with_context(|| context.to_string())?;
    if output.status.success() {
        return Ok(());
    }
    bail!(
        "{context}: {}",
        String::from_utf8_lossy(&output.stderr).trim()
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use scope_domain::runs::resources::JobResources;
    use scope_domain::runs::workflow::{
        ContainerSpec, RunnerSelector, WorkflowJob, WorkflowJobId, WorkflowStep,
    };

    #[test]
    fn container_restarts_select_read_only_step_programs_without_log_markers() {
        let job = WorkflowJob::new(
            WorkflowJobId::parse("checks").unwrap(),
            vec![],
            RunnerSelector::Any,
            ContainerSpec::new("alpine:3.20").unwrap(),
            JobResources::new(1_000, 1024 * 1024 * 1024).unwrap(),
            60,
            Vec::new(),
            vec![
                WorkflowStep::new("first", "printf one").unwrap(),
                WorkflowStep::new("second", "printf two").unwrap(),
            ],
        )
        .unwrap();
        let root =
            std::env::temp_dir().join(format!("scope-step-program-test-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir(&root).unwrap();

        let programs = write_step_programs(&root, &job).unwrap();
        assert_eq!(
            fs::read_to_string(programs.join("step-0.sh")).unwrap(),
            "printf one"
        );
        assert_eq!(
            fs::read_to_string(programs.join("step-1.sh")).unwrap(),
            "printf two"
        );
        assert!(!programs.join("supervisor.sh").exists());
        select_container_step(&programs, "prepare", 1, "nonce").unwrap();
        assert_eq!(
            fs::read_to_string(programs.join("current")).unwrap(),
            "prepare 1 nonce\n"
        );

        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn storage_pressure_overrides_a_racing_successful_exit() {
        assert_eq!(exit_code_after_storage_guard(0, true), 1);
        assert_eq!(exit_code_after_storage_guard(7, false), 7);
    }
}
