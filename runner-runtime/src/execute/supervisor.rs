use super::{
    ExecutionSink,
    output::{OutputCapture, OutputNotice, UploadPolicy},
    process::StepProcess,
};
use anyhow::Context as _;
use scope_domain::runs::workflow::WorkflowJob;
use std::{path::Path, sync::Arc, thread, time::Duration, time::Instant};

const HEARTBEAT_INTERVAL: Duration = Duration::from_secs(10);
const SUPERVISOR_POLL_INTERVAL: Duration = Duration::from_millis(50);
const TERMINATION_GRACE: Duration = Duration::from_secs(2);

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum ExecutionOutcome {
    Succeeded { logs_truncated: bool },
    Terminal,
}

#[derive(Clone, Copy)]
pub(crate) struct SupervisorOptions {
    heartbeat_interval: Duration,
    poll_interval: Duration,
    termination_grace: Duration,
    upload_policy: UploadPolicy,
    timeout: Option<Duration>,
}

impl Default for SupervisorOptions {
    fn default() -> Self {
        Self {
            heartbeat_interval: HEARTBEAT_INTERVAL,
            poll_interval: SUPERVISOR_POLL_INTERVAL,
            termination_grace: TERMINATION_GRACE,
            upload_policy: UploadPolicy::default(),
            timeout: None,
        }
    }
}

pub(crate) fn run_steps<S: ExecutionSink>(
    sink: S,
    job: &WorkflowJob,
    workspace: &Path,
) -> anyhow::Result<ExecutionOutcome> {
    run_steps_with_options(sink, job, workspace, SupervisorOptions::default())
}

pub(crate) fn run_steps_with_options<S: ExecutionSink>(
    sink: S,
    job: &WorkflowJob,
    workspace: &Path,
    options: SupervisorOptions,
) -> anyhow::Result<ExecutionOutcome> {
    let sink = Arc::new(sink);
    let timeout = options
        .timeout
        .unwrap_or_else(|| Duration::from_secs(job.timeout_seconds()));
    let deadline = Instant::now() + timeout;
    let mut next_sequence = 1_u64;
    let mut logs_truncated = false;

    for (index, step) in job.steps().iter().enumerate() {
        let index = u32::try_from(index).context("step index overflow")?;
        match sink.start_step(index) {
            Ok(true) => {
                complete_canceled_or_abandon(sink.as_ref(), logs_truncated)?;
                return Ok(ExecutionOutcome::Terminal);
            }
            Ok(false) => {}
            Err(error) => return abandon_after_error(sink.as_ref(), error),
        }

        let mut process = match StepProcess::spawn(step.run(), job, workspace) {
            Ok(process) => process,
            Err(error) => {
                return abandon_after_error(
                    sink.as_ref(),
                    error.context(format!("start step {}", step.name())),
                );
            }
        };
        let stdout = match process.take_stdout() {
            Ok(stdout) => stdout,
            Err(error) => {
                return cleanup_after_error(
                    sink.as_ref(),
                    process,
                    None,
                    options.termination_grace,
                    error,
                );
            }
        };
        let stderr = match process.take_stderr() {
            Ok(stderr) => stderr,
            Err(error) => {
                return cleanup_after_error(
                    sink.as_ref(),
                    process,
                    None,
                    options.termination_grace,
                    error,
                );
            }
        };
        let capture = match OutputCapture::start(
            stdout,
            stderr,
            Arc::clone(&sink),
            index,
            next_sequence,
            logs_truncated,
            options.upload_policy,
        ) {
            Ok(capture) => capture,
            Err(error) => {
                return cleanup_after_error(
                    sink.as_ref(),
                    process,
                    None,
                    options.termination_grace,
                    error,
                );
            }
        };

        let mut capture = Some(capture);
        let mut process = Some(process);
        let mut status = None;
        let mut output = None;
        let mut next_heartbeat = Instant::now() + options.heartbeat_interval;
        let mut group_kill_at = None;
        let mut group_killed = false;

        loop {
            while let Some(notice) = capture.as_ref().and_then(OutputCapture::try_notice) {
                match notice {
                    OutputNotice::Truncated => logs_truncated = true,
                    OutputNotice::Finished(summary) => {
                        logs_truncated |= summary.logs_truncated;
                        output = Some(summary);
                    }
                    OutputNotice::Failed(error) => {
                        return cleanup_after_output_error(
                            sink.as_ref(),
                            process.take().expect("step process exists"),
                            capture.take().expect("output capture exists"),
                            options.termination_grace,
                            error,
                        );
                    }
                }
            }

            if status.is_none() {
                match process.as_mut().expect("step process exists").try_wait() {
                    Ok(Some(exit_status)) => {
                        status = Some(exit_status);
                        if let Err(error) = process
                            .as_ref()
                            .expect("step process exists")
                            .signal_terminate()
                        {
                            return cleanup_after_error(
                                sink.as_ref(),
                                process.take().expect("step process exists"),
                                capture.take(),
                                options.termination_grace,
                                error,
                            );
                        }
                        group_kill_at = Some(Instant::now() + options.termination_grace);
                    }
                    Ok(None) => {}
                    Err(error) => {
                        return cleanup_after_error(
                            sink.as_ref(),
                            process.take().expect("step process exists"),
                            capture.take(),
                            options.termination_grace,
                            error,
                        );
                    }
                }
            }

            if !group_killed && group_kill_at.is_some_and(|kill_at| Instant::now() >= kill_at) {
                if let Err(error) = process.as_ref().expect("step process exists").signal_kill() {
                    return cleanup_after_error(
                        sink.as_ref(),
                        process.take().expect("step process exists"),
                        capture.take(),
                        options.termination_grace,
                        error,
                    );
                }
                group_killed = true;
                if output.is_none() {
                    capture.as_ref().expect("output capture exists").stop();
                }
            }

            if let Some(exit_status) = status
                && let Some(summary) = output.take()
            {
                let final_status = match process.take().expect("step process exists").finish() {
                    Ok(final_status) => final_status,
                    Err(error) => {
                        if let Err(capture_error) =
                            capture.take().expect("output capture exists").join()
                        {
                            eprintln!("runtime failed to join output capture: {capture_error:#}");
                        }
                        return abandon_after_error(sink.as_ref(), error);
                    }
                };
                if let Err(error) = capture.take().expect("output capture exists").join() {
                    return abandon_after_error(sink.as_ref(), error);
                }
                debug_assert_eq!(exit_status, final_status);
                next_sequence = summary.next_sequence;
                logs_truncated |= summary.logs_truncated;
                let exit_code = final_status.code().unwrap_or(128);
                if let Err(error) = sink.complete_step(index, exit_code, logs_truncated) {
                    return abandon_after_error(sink.as_ref(), error);
                }
                if exit_code != 0 {
                    return Ok(ExecutionOutcome::Terminal);
                }
                break;
            }

            let now = Instant::now();
            if now >= deadline {
                let logs_truncated = terminate_step(
                    process.take().expect("step process exists"),
                    capture.take().expect("output capture exists"),
                    options.termination_grace,
                    logs_truncated,
                );
                if let Err(error) = sink.complete_timeout(logs_truncated) {
                    return abandon_after_error(sink.as_ref(), error);
                }
                return Ok(ExecutionOutcome::Terminal);
            }
            if now >= next_heartbeat {
                match sink.heartbeat() {
                    Ok(true) => {
                        let logs_truncated = terminate_step(
                            process.take().expect("step process exists"),
                            capture.take().expect("output capture exists"),
                            options.termination_grace,
                            logs_truncated,
                        );
                        complete_canceled_or_abandon(sink.as_ref(), logs_truncated)?;
                        return Ok(ExecutionOutcome::Terminal);
                    }
                    Ok(false) => next_heartbeat = now + options.heartbeat_interval,
                    Err(error) => {
                        return cleanup_after_error(
                            sink.as_ref(),
                            process.take().expect("step process exists"),
                            capture.take(),
                            options.termination_grace,
                            error,
                        );
                    }
                }
            }
            thread::sleep(options.poll_interval);
        }
    }

    Ok(ExecutionOutcome::Succeeded { logs_truncated })
}

fn terminate_step(
    process: StepProcess,
    capture: OutputCapture,
    grace: Duration,
    mut logs_truncated: bool,
) -> bool {
    if let Err(error) = process.terminate_and_wait(grace) {
        eprintln!("runtime failed to clean up step process: {error:#}");
    }
    capture.stop();
    match capture.wait() {
        Ok(summary) => logs_truncated |= summary.logs_truncated,
        Err(error) => eprintln!("runtime failed to finish output capture: {error:#}"),
    }
    logs_truncated
}

fn cleanup_after_error<S: ExecutionSink, T>(
    sink: &S,
    process: StepProcess,
    capture: Option<OutputCapture>,
    grace: Duration,
    error: anyhow::Error,
) -> anyhow::Result<T> {
    if let Err(cleanup_error) = process.terminate_and_wait(grace) {
        eprintln!("runtime failed to clean up step process: {cleanup_error:#}");
    }
    if let Some(capture) = capture.as_ref() {
        capture.stop();
    }
    if let Some(capture) = capture
        && let Err(capture_error) = capture.wait()
    {
        eprintln!("runtime failed to finish output capture: {capture_error:#}");
    }
    abandon_after_error(sink, error)
}

fn cleanup_after_output_error<S: ExecutionSink, T>(
    sink: &S,
    process: StepProcess,
    capture: OutputCapture,
    grace: Duration,
    error: anyhow::Error,
) -> anyhow::Result<T> {
    if let Err(cleanup_error) = process.terminate_and_wait(grace) {
        eprintln!("runtime failed to clean up step process: {cleanup_error:#}");
    }
    capture.stop();
    if let Err(capture_error) = capture.join() {
        eprintln!("runtime failed to join output capture: {capture_error:#}");
    }
    abandon_after_error(sink, error)
}

fn complete_canceled_or_abandon<S: ExecutionSink>(
    sink: &S,
    logs_truncated: bool,
) -> anyhow::Result<()> {
    match sink.complete_canceled(logs_truncated) {
        Ok(()) => Ok(()),
        Err(error) => abandon_after_error(sink, error),
    }
}

fn abandon_after_error<S: ExecutionSink, T>(sink: &S, error: anyhow::Error) -> anyhow::Result<T> {
    if let Err(abandon_error) = sink.abandon() {
        eprintln!("runtime failed to abandon attempt after execution error: {abandon_error:#}");
    }
    Err(error)
}

#[cfg(test)]
impl SupervisorOptions {
    pub(crate) fn for_test(timeout: Duration) -> Self {
        Self {
            heartbeat_interval: Duration::from_millis(25),
            poll_interval: Duration::from_millis(5),
            termination_grace: Duration::from_millis(25),
            upload_policy: UploadPolicy {
                attempts: 2,
                retry_delay: Duration::from_millis(5),
            },
            timeout: Some(timeout),
        }
    }
}
