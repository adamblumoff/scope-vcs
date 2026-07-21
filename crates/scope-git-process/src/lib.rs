use std::{
    io::{ErrorKind, Read, Write},
    process::{Child, Command, Output, Stdio},
    sync::mpsc,
    thread,
    time::{Duration, Instant},
};

pub const STDERR_DIAGNOSTIC_BYTES: usize = 8 * 1024;

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

fn read_stdout(
    mut stdout: impl Read,
    max_bytes: Option<usize>,
    limit_sender: mpsc::Sender<()>,
) -> std::io::Result<Vec<u8>> {
    let mut retained = Vec::new();
    let mut buffer = [0_u8; 8 * 1024];
    let mut limit_reported = false;
    loop {
        let read = stdout.read(&mut buffer)?;
        if read == 0 {
            return Ok(retained);
        }
        match max_bytes {
            Some(max_bytes) => {
                let max_retained = max_bytes.saturating_add(1);
                let remaining = max_retained.saturating_sub(retained.len());
                if remaining > 0 {
                    retained.extend_from_slice(&buffer[..read.min(remaining)]);
                }
                if retained.len() > max_bytes && !limit_reported {
                    let _ = limit_sender.send(());
                    limit_reported = true;
                }
            }
            None => retained.extend_from_slice(&buffer[..read]),
        }
    }
}

fn read_stderr_diagnostic(mut stderr: impl Read, max_bytes: usize) -> std::io::Result<Vec<u8>> {
    let max_retained = max_bytes.saturating_add(1);
    let mut retained = Vec::new();
    let mut buffer = [0_u8; 8 * 1024];
    loop {
        let read = stderr.read(&mut buffer)?;
        if read == 0 {
            return Ok(retained);
        }
        let remaining = max_retained.saturating_sub(retained.len());
        if remaining > 0 {
            retained.extend_from_slice(&buffer[..read.min(remaining)]);
        }
    }
}

fn join_reader(
    handle: thread::JoinHandle<std::io::Result<Vec<u8>>>,
    action: &str,
) -> Result<Vec<u8>, ProcessError> {
    handle
        .join()
        .map_err(|_| ProcessError::ThreadPanicked {
            action: action.to_string(),
        })?
        .map_err(|source| ProcessError::Io {
            action: action.to_string(),
            source,
        })
}

fn join_writer(
    handle: Option<thread::JoinHandle<std::io::Result<()>>>,
    action: &str,
) -> Result<(), ProcessError> {
    let Some(handle) = handle else {
        return Ok(());
    };
    match handle.join().map_err(|_| ProcessError::ThreadPanicked {
        action: action.to_string(),
    })? {
        Ok(()) => Ok(()),
        Err(source) if source.kind() == ErrorKind::BrokenPipe => Ok(()),
        Err(source) => Err(ProcessError::Io {
            action: action.to_string(),
            source,
        }),
    }
}

fn diagnostic_suffix(stderr: &[u8], max_bytes: usize) -> String {
    let message = truncated_stderr(stderr, max_bytes);
    if message.is_empty() {
        String::new()
    } else {
        format!(": {message}")
    }
}

pub fn truncated_stderr(stderr: &[u8], max_bytes: usize) -> String {
    let mut message = String::from_utf8_lossy(stderr).trim().to_string();
    if message.len() > max_bytes {
        let mut end = 0;
        for (index, character) in message.char_indices() {
            let next = index + character.len_utf8();
            if next > max_bytes {
                break;
            }
            end = next;
        }
        message.truncate(end);
        message.push_str("...");
    }
    message
}

#[cfg(unix)]
fn configure_process_group(command: &mut Command) {
    use std::os::unix::process::CommandExt;
    command.process_group(0);
}

#[cfg(not(unix))]
fn configure_process_group(_command: &mut Command) {}

#[cfg(unix)]
fn terminate_and_reap(child: &mut Child) {
    if let Ok(process_group) = i32::try_from(child.id()) {
        // SAFETY: a negative, non-zero pid targets only the process group created for this child.
        unsafe {
            libc::kill(-process_group, libc::SIGKILL);
        }
    }
    let _ = child.kill();
    let _ = child.wait();
}

#[cfg(not(unix))]
fn terminate_and_reap(child: &mut Child) {
    let _ = child.kill();
    let _ = child.wait();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stdout_limit_accepts_exact_boundary_and_rejects_one_more_byte() {
        let mut exact = Command::new("sh");
        exact.arg("-c").arg("printf 1234");
        let output = run(
            &mut exact,
            None,
            ProcessLimits::new(Duration::from_secs(1)).with_max_stdout_bytes(4),
            "exact output",
        )
        .unwrap();
        assert_eq!(output.stdout, b"1234");

        let mut oversized = Command::new("sh");
        oversized.arg("-c").arg("printf 12345");
        let error = run(
            &mut oversized,
            None,
            ProcessLimits::new(Duration::from_secs(1)).with_max_stdout_bytes(4),
            "oversized output",
        )
        .unwrap_err();
        assert!(error.is_stdout_limit());
        assert_eq!(
            error.to_string(),
            "oversized output stdout exceeded 4 bytes"
        );
    }

    #[test]
    fn timeout_covers_blocked_stdin_write() {
        let mut command = Command::new("sh");
        command.arg("-c").arg("sleep 5");
        let started_at = Instant::now();
        let error = run(
            &mut command,
            Some(vec![b'x'; 8 * 1024 * 1024]),
            ProcessLimits::new(Duration::from_millis(100)),
            "blocked input",
        )
        .unwrap_err();

        assert!(error.is_timeout());
        assert!(started_at.elapsed() < Duration::from_secs(2));
    }

    #[cfg(unix)]
    #[test]
    fn stdout_limit_kills_descendants_holding_output_pipes() {
        let mut command = Command::new("sh");
        command.arg("-c").arg("(sleep 5) & printf 12345; sleep 5");
        let started_at = Instant::now();
        let error = run(
            &mut command,
            None,
            ProcessLimits::new(Duration::from_millis(250)).with_max_stdout_bytes(4),
            "descendant output limit",
        )
        .unwrap_err();

        assert!(error.is_stdout_limit());
        assert!(started_at.elapsed() < Duration::from_secs(2));
    }

    #[cfg(unix)]
    #[test]
    fn timeout_kills_descendants_holding_output_pipes() {
        let mut command = Command::new("sh");
        command.arg("-c").arg("(sleep 5) & sleep 5");
        let started_at = Instant::now();
        let error = run(
            &mut command,
            None,
            ProcessLimits::new(Duration::from_millis(25)),
            "descendant timeout",
        )
        .unwrap_err();

        assert!(error.is_timeout());
        assert!(started_at.elapsed() < Duration::from_secs(2));
    }
}
