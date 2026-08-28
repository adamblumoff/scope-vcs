use crate::{
    STDERR_DIAGNOSTIC_BYTES, configure_process_group,
    lifecycle::terminate_and_reap,
    stdio::{diagnostic_suffix, join_reader, join_writer, read_stderr_diagnostic, read_stdout},
};
use std::{
    io::{Read, Write},
    process::{Child, Command, ExitStatus, Output, Stdio},
    sync::mpsc,
    thread,
    time::{Duration, Instant},
};

#[derive(Clone, Copy, Debug)]
pub struct ProcessLimits {
    timeout: Duration,
    max_stdout_bytes: Option<usize>,
}

impl ProcessLimits {
    pub fn new(timeout: Duration) -> Self {
        Self {
            timeout,
            max_stdout_bytes: None,
        }
    }

    pub fn with_max_stdout_bytes(mut self, max_stdout_bytes: usize) -> Self {
        self.max_stdout_bytes = Some(max_stdout_bytes);
        self
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ProcessError {
    #[error("failed to run {action}: {source}")]
    Spawn {
        action: String,
        #[source]
        source: std::io::Error,
    },
    #[error("{action} {pipe} pipe is unavailable")]
    PipeUnavailable { action: String, pipe: &'static str },
    #[error("{action} process I/O failed: {source}")]
    Io {
        action: String,
        #[source]
        source: std::io::Error,
    },
    #[error("{action} process I/O thread panicked")]
    ThreadPanicked { action: String },
    #[error("{action} timed out after {timeout_ms}ms{diagnostic}")]
    TimedOut {
        action: String,
        timeout_ms: u128,
        diagnostic: String,
    },
    #[error("{action} stdout exceeded {max_stdout_bytes} bytes{diagnostic}")]
    StdoutLimitExceeded {
        action: String,
        max_stdout_bytes: usize,
        diagnostic: String,
    },
}

#[derive(Debug)]
pub struct StreamedOutput<T> {
    pub status: ExitStatus,
    pub value: T,
    pub stderr: Vec<u8>,
}

#[derive(Debug, thiserror::Error)]
pub enum StreamingProcessError<E> {
    #[error(transparent)]
    Process(#[from] ProcessError),
    #[error("process stdout consumer failed: {0}")]
    Consumer(E),
}

impl ProcessError {
    pub fn is_timeout(&self) -> bool {
        matches!(self, Self::TimedOut { .. })
    }

    pub fn is_stdout_limit(&self) -> bool {
        matches!(self, Self::StdoutLimitExceeded { .. })
    }
}

pub fn run(
    command: &mut Command,
    input: Option<Vec<u8>>,
    limits: ProcessLimits,
    action: &str,
) -> Result<Output, ProcessError> {
    if input.is_some() {
        command.stdin(Stdio::piped());
    } else {
        command.stdin(Stdio::null());
    }
    configure_process_group(command);
    let mut child = command
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|source| ProcessError::Spawn {
            action: action.to_string(),
            source,
        })?;
    let stdin_writer = if let Some(input) = input {
        let mut child_stdin = child
            .stdin
            .take()
            .ok_or_else(|| ProcessError::PipeUnavailable {
                action: action.to_string(),
                pipe: "stdin",
            })?;
        Some(thread::spawn(move || {
            child_stdin.write_all(&input)?;
            child_stdin.flush()
        }))
    } else {
        None
    };
    wait_for_output(child, stdin_writer, limits, action)
}

/// Runs a child while copying a caller-owned reader into stdin incrementally.
///
/// This preserves the same timeout, bounded-output, and process-tree cleanup
/// behavior as [`run`] without requiring the complete input in memory.
pub fn run_with_stdin_reader<R>(
    command: &mut Command,
    mut input: R,
    limits: ProcessLimits,
    action: &str,
) -> Result<Output, ProcessError>
where
    R: Read + Send + 'static,
{
    command.stdin(Stdio::piped());
    configure_process_group(command);
    let mut child = command
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|source| ProcessError::Spawn {
            action: action.to_string(),
            source,
        })?;
    let mut child_stdin = child
        .stdin
        .take()
        .ok_or_else(|| ProcessError::PipeUnavailable {
            action: action.to_string(),
            pipe: "stdin",
        })?;
    let stdin_writer = thread::spawn(move || {
        std::io::copy(&mut input, &mut child_stdin)?;
        child_stdin.flush()
    });
    wait_for_output(child, Some(stdin_writer), limits, action)
}

/// Runs a child while a caller-owned consumer drains stdout incrementally.
///
/// The consumer executes on a dedicated thread so this function can retain the
/// existing timeout and process-group cleanup guarantees. Unlike [`run`], this
/// path never collects stdout into a `Vec` owned by the process runner.
pub fn run_with_stdout<T, E, F>(
    command: &mut Command,
    input: Option<Vec<u8>>,
    limits: ProcessLimits,
    action: &str,
    consume: F,
) -> Result<StreamedOutput<T>, StreamingProcessError<E>>
where
    T: Send + 'static,
    E: Send + 'static,
    F: FnOnce(Box<dyn Read + Send>) -> Result<T, E> + Send + 'static,
{
    if input.is_some() {
        command.stdin(Stdio::piped());
    } else {
        command.stdin(Stdio::null());
    }
    configure_process_group(command);
    let mut child = command
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|source| ProcessError::Spawn {
            action: action.to_string(),
            source,
        })?;
    let stdin_writer = if let Some(input) = input {
        let mut child_stdin = child
            .stdin
            .take()
            .ok_or_else(|| ProcessError::PipeUnavailable {
                action: action.to_string(),
                pipe: "stdin",
            })?;
        Some(thread::spawn(move || {
            child_stdin.write_all(&input)?;
            child_stdin.flush()
        }))
    } else {
        None
    };
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| ProcessError::PipeUnavailable {
            action: action.to_string(),
            pipe: "stdout",
        })?;
    let stderr = child
        .stderr
        .take()
        .ok_or_else(|| ProcessError::PipeUnavailable {
            action: action.to_string(),
            pipe: "stderr",
        })?;
    let mut stdout_consumer = Some(thread::spawn(move || consume(Box::new(stdout))));
    let stderr_reader =
        thread::spawn(move || read_stderr_diagnostic(stderr, STDERR_DIAGNOSTIC_BYTES));

    let started_at = Instant::now();
    let mut status = None;
    let mut value = None;
    loop {
        if stdout_consumer
            .as_ref()
            .is_some_and(thread::JoinHandle::is_finished)
        {
            match stdout_consumer
                .take()
                .expect("finished stdout consumer must exist")
                .join()
            {
                Ok(Ok(consumed)) => value = Some(consumed),
                Ok(Err(error)) => {
                    terminate_and_reap(&mut child);
                    let _ = join_writer(stdin_writer, action);
                    let _ = join_reader(stderr_reader, action);
                    return Err(StreamingProcessError::Consumer(error));
                }
                Err(_) => {
                    terminate_and_reap(&mut child);
                    let _ = join_writer(stdin_writer, action);
                    let _ = join_reader(stderr_reader, action);
                    return Err(ProcessError::ThreadPanicked {
                        action: action.to_string(),
                    }
                    .into());
                }
            }
        }
        if status.is_none() {
            status = child.try_wait().map_err(|source| ProcessError::Io {
                action: action.to_string(),
                source,
            })?;
        }
        let stdin_done = stdin_writer
            .as_ref()
            .is_none_or(thread::JoinHandle::is_finished);
        if value.is_some() && stderr_reader.is_finished() && stdin_done && status.is_some() {
            break;
        }
        if started_at.elapsed() >= limits.timeout {
            terminate_and_reap(&mut child);
            let _ = join_writer(stdin_writer, action);
            if let Some(stdout_consumer) = stdout_consumer {
                let _ = stdout_consumer.join();
            }
            let stderr = join_reader(stderr_reader, action).unwrap_or_default();
            return Err(ProcessError::TimedOut {
                action: action.to_string(),
                timeout_ms: limits.timeout.as_millis(),
                diagnostic: diagnostic_suffix(&stderr, STDERR_DIAGNOSTIC_BYTES),
            }
            .into());
        }
        let remaining = limits.timeout.saturating_sub(started_at.elapsed());
        thread::sleep(remaining.min(Duration::from_millis(1)));
    }

    join_writer(stdin_writer, action)?;
    let stderr = join_reader(stderr_reader, action)?;
    Ok(StreamedOutput {
        status: status.expect("completed child must have an exit status"),
        value: value.expect("completed stdout consumer must return a value"),
        stderr,
    })
}

fn wait_for_output(
    mut child: Child,
    stdin_writer: Option<thread::JoinHandle<std::io::Result<()>>>,
    limits: ProcessLimits,
    action: &str,
) -> Result<Output, ProcessError> {
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| ProcessError::PipeUnavailable {
            action: action.to_string(),
            pipe: "stdout",
        })?;
    let stderr = child
        .stderr
        .take()
        .ok_or_else(|| ProcessError::PipeUnavailable {
            action: action.to_string(),
            pipe: "stderr",
        })?;
    let (stdout_limit_sender, stdout_limit_receiver) = mpsc::channel();
    let stdout_reader =
        thread::spawn(move || read_stdout(stdout, limits.max_stdout_bytes, stdout_limit_sender));
    let stderr_reader =
        thread::spawn(move || read_stderr_diagnostic(stderr, STDERR_DIAGNOSTIC_BYTES));

    let started_at = Instant::now();
    let mut status = None;
    let status = loop {
        if stdout_limit_receiver.try_recv().is_ok() {
            terminate_and_reap(&mut child);
            let _ = join_writer(stdin_writer, action);
            let _ = join_reader(stdout_reader, action);
            let stderr = join_reader(stderr_reader, action).unwrap_or_default();
            return Err(ProcessError::StdoutLimitExceeded {
                action: action.to_string(),
                max_stdout_bytes: limits
                    .max_stdout_bytes
                    .expect("stdout limit signal requires a configured limit"),
                diagnostic: diagnostic_suffix(&stderr, STDERR_DIAGNOSTIC_BYTES),
            });
        }
        if status.is_none() {
            status = child.try_wait().map_err(|source| ProcessError::Io {
                action: action.to_string(),
                source,
            })?;
        }
        let stdin_done = stdin_writer
            .as_ref()
            .is_none_or(thread::JoinHandle::is_finished);
        if stdout_reader.is_finished()
            && stderr_reader.is_finished()
            && stdin_done
            && let Some(status) = status.take()
        {
            break status;
        }
        if started_at.elapsed() >= limits.timeout {
            terminate_and_reap(&mut child);
            let _ = join_writer(stdin_writer, action);
            let _ = join_reader(stdout_reader, action);
            let stderr = join_reader(stderr_reader, action).unwrap_or_default();
            return Err(ProcessError::TimedOut {
                action: action.to_string(),
                timeout_ms: limits.timeout.as_millis(),
                diagnostic: diagnostic_suffix(&stderr, STDERR_DIAGNOSTIC_BYTES),
            });
        }
        let remaining = limits.timeout.saturating_sub(started_at.elapsed());
        thread::sleep(remaining.min(Duration::from_millis(1)));
    };

    join_writer(stdin_writer, action)?;
    let stdout = join_reader(stdout_reader, action)?;
    let stderr = join_reader(stderr_reader, action)?;
    if let Some(max_stdout_bytes) = limits.max_stdout_bytes
        && stdout.len() > max_stdout_bytes
    {
        return Err(ProcessError::StdoutLimitExceeded {
            action: action.to_string(),
            max_stdout_bytes,
            diagnostic: diagnostic_suffix(&stderr, STDERR_DIAGNOSTIC_BYTES),
        });
    }
    Ok(Output {
        status,
        stdout,
        stderr,
    })
}
