use std::{
    collections::BTreeSet,
    fs,
    io::{ErrorKind, Read, Write},
    path::Path,
    process::{Child, Command, ExitStatus, Output, Stdio},
    sync::{
        atomic::{AtomicI32, Ordering},
        mpsc,
    },
    thread,
    time::{Duration, Instant},
};

pub const STDERR_DIAGNOSTIC_BYTES: usize = 8 * 1024;

#[cfg(unix)]
const INTERNAL_REAPER_CHILD_ENV: &str = "SCOPE_INTERNAL_REAPER_CHILD";

#[cfg(unix)]
static PENDING_REAPER_SIGNAL: AtomicI32 = AtomicI32::new(0);

/// When the service itself is PID 1, respawn it behind a minimal init process.
///
/// Railway can override image entrypoints, so relying on an external init is
/// not sufficient. The parent created here owns only the service process and
/// adopted descendants; the service remains free to wait on its direct Git
/// children without a competing global `waitpid` loop.
#[cfg(unix)]
pub fn install_pid1_reaper_if_needed() -> std::io::Result<()> {
    use std::os::unix::process::CommandExt;

    if std::process::id() != 1 || std::env::var_os(INTERNAL_REAPER_CHILD_ENV).is_some() {
        return Ok(());
    }

    let executable = std::env::current_exe()?;
    let mut command = Command::new(executable);
    command
        .args(std::env::args_os().skip(1))
        .env(INTERNAL_REAPER_CHILD_ENV, "1")
        .process_group(0);
    let child = command.spawn()?;
    install_reaper_signal_handlers()?;
    reap_service_process(child.id())
}

#[cfg(not(unix))]
pub fn install_pid1_reaper_if_needed() -> std::io::Result<()> {
    Ok(())
}

#[cfg(unix)]
fn install_reaper_signal_handlers() -> std::io::Result<()> {
    unsafe extern "C" fn remember_signal(signal: libc::c_int) {
        PENDING_REAPER_SIGNAL.store(signal, Ordering::SeqCst);
    }

    for signal in [libc::SIGTERM, libc::SIGINT, libc::SIGHUP] {
        // SAFETY: sigaction is initialized before use and the handler performs
        // only an async-signal-safe atomic store.
        let result = unsafe {
            let mut action: libc::sigaction = std::mem::zeroed();
            action.sa_sigaction = remember_signal as *const () as usize;
            action.sa_flags = 0;
            libc::sigemptyset(&mut action.sa_mask);
            libc::sigaction(signal, &action, std::ptr::null_mut())
        };
        if result == -1 {
            return Err(std::io::Error::last_os_error());
        }
    }
    Ok(())
}

#[cfg(unix)]
fn reap_service_process(service_pid: u32) -> std::io::Result<()> {
    let service_pid = i32::try_from(service_pid)
        .map_err(|_| std::io::Error::other("service process id exceeds i32"))?;
    loop {
        forward_pending_signal(service_pid);
        let mut status = 0;
        // SAFETY: waitpid writes only to the supplied status integer. PID -1
        // is intentional because this process exists solely to reap children.
        let reaped = unsafe { libc::waitpid(-1, &mut status, 0) };
        if reaped == -1 {
            let error = std::io::Error::last_os_error();
            if error.kind() == ErrorKind::Interrupted {
                continue;
            }
            return Err(error);
        }
        if reaped != service_pid {
            continue;
        }

        // The service has stopped. Kill anything still in its process group,
        // reap every adopted descendant, then preserve the service exit code.
        // SAFETY: the negative pid targets only the service process group.
        unsafe {
            libc::kill(-service_pid, libc::SIGKILL);
        }
        drain_adopted_descendants();
        std::process::exit(wait_status_exit_code(status));
    }
}

#[cfg(unix)]
fn forward_pending_signal(service_pid: i32) {
    let signal = PENDING_REAPER_SIGNAL.swap(0, Ordering::SeqCst);
    if signal != 0 {
        // SAFETY: the negative pid targets only the service process group.
        unsafe {
            libc::kill(-service_pid, signal);
        }
    }
}

#[cfg(unix)]
fn drain_adopted_descendants() {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        reap_exited_descendants();
        let descendants = child_process_ids().unwrap_or_default();
        if descendants.is_empty() {
            return;
        }

        for process_id in descendants {
            let Ok(process_id) = i32::try_from(process_id) else {
                continue;
            };
            // Adopted Git commands lead their own process groups, while a
            // non-leader descendant still needs a direct kill. Both calls are
            // intentionally best-effort during container shutdown.
            unsafe {
                libc::kill(-process_id, libc::SIGKILL);
                libc::kill(process_id, libc::SIGKILL);
            }
        }
        if Instant::now() >= deadline {
            return;
        }
        thread::sleep(Duration::from_millis(1));
    }
}

#[cfg(unix)]
fn reap_exited_descendants() {
    loop {
        let mut status = 0;
        // SAFETY: after the service exits, every remaining descendant is
        // owned by this dedicated reaper. WNOHANG prevents shutdown hangs.
        let reaped = unsafe { libc::waitpid(-1, &mut status, libc::WNOHANG) };
        if reaped <= 0 {
            return;
        }
    }
}

#[cfg(unix)]
fn wait_status_exit_code(status: i32) -> i32 {
    if libc::WIFEXITED(status) {
        libc::WEXITSTATUS(status)
    } else if libc::WIFSIGNALED(status) {
        128 + libc::WTERMSIG(status)
    } else {
        1
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ProcessSnapshot {
    pub process_id: u32,
    pub parent_process_id: Option<usize>,
    pub threads: Option<usize>,
    pub open_file_descriptors: Option<usize>,
    pub child_processes: Option<usize>,
    pub zombie_child_processes: Option<usize>,
    pub cgroup_pids_current: Option<usize>,
    pub cgroup_pids_max: Option<usize>,
    pub cgroup_pids_unlimited: bool,
}

pub fn current_process_snapshot() -> ProcessSnapshot {
    let status = fs::read_to_string("/proc/self/status").ok();
    let pids_max = fs::read_to_string("/sys/fs/cgroup/pids.max").ok();
    let children = child_process_counts();
    ProcessSnapshot {
        process_id: std::process::id(),
        parent_process_id: status
            .as_deref()
            .and_then(|contents| parse_status_usize(contents, "PPid")),
        threads: status
            .as_deref()
            .and_then(|contents| parse_status_usize(contents, "Threads")),
        open_file_descriptors: count_directory_entries("/proc/self/fd"),
        child_processes: children.map(|counts| counts.total),
        zombie_child_processes: children.map(|counts| counts.zombies),
        cgroup_pids_current: read_usize("/sys/fs/cgroup/pids.current"),
        cgroup_pids_max: pids_max.as_deref().and_then(parse_trimmed_usize),
        cgroup_pids_unlimited: pids_max
            .as_deref()
            .is_some_and(|value| value.trim() == "max"),
    }
}

fn parse_status_usize(contents: &str, name: &str) -> Option<usize> {
    contents.lines().find_map(|line| {
        let (key, value) = line.split_once(':')?;
        (key == name).then(|| parse_trimmed_usize(value)).flatten()
    })
}

fn parse_trimmed_usize(value: &str) -> Option<usize> {
    value.trim().parse().ok()
}

fn read_usize(path: impl AsRef<Path>) -> Option<usize> {
    fs::read_to_string(path)
        .ok()
        .as_deref()
        .and_then(parse_trimmed_usize)
}

fn count_directory_entries(path: impl AsRef<Path>) -> Option<usize> {
    Some(fs::read_dir(path).ok()?.filter_map(Result::ok).count())
}

#[derive(Clone, Copy)]
struct ChildProcessCounts {
    total: usize,
    zombies: usize,
}

fn child_process_counts() -> Option<ChildProcessCounts> {
    let children = child_process_ids()?;
    let zombies = children
        .iter()
        .filter(|pid| child_process_state(**pid).as_deref() == Some("Z"))
        .count();
    Some(ChildProcessCounts {
        total: children.len(),
        zombies,
    })
}

fn child_process_ids() -> Option<BTreeSet<usize>> {
    let mut children = BTreeSet::new();
    for task in fs::read_dir("/proc/self/task").ok()?.filter_map(Result::ok) {
        let contents = fs::read_to_string(task.path().join("children")).unwrap_or_default();
        children.extend(contents.split_whitespace().filter_map(parse_trimmed_usize));
    }
    Some(children)
}

fn child_process_state(pid: usize) -> Option<String> {
    let stat = fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    let command_end = stat.rfind(')')?;
    stat.get(command_end + 2..)?
        .split_whitespace()
        .next()
        .map(str::to_string)
}

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
pub fn configure_process_group(command: &mut Command) {
    use std::os::unix::process::CommandExt;
    command.process_group(0);
}

#[cfg(not(unix))]
pub fn configure_process_group(_command: &mut Command) {}

#[cfg(unix)]
pub fn kill_process_group(process_id: u32) {
    if let Ok(process_group) = i32::try_from(process_id) {
        // SAFETY: a negative, non-zero pid targets only the process group created for this child.
        unsafe {
            libc::kill(-process_group, libc::SIGKILL);
        }
    }
}

#[cfg(not(unix))]
pub fn kill_process_group(_process_id: u32) {}

#[cfg(unix)]
fn terminate_and_reap(child: &mut Child) {
    kill_process_group(child.id());
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
    fn parses_linux_process_status_values() {
        let status = "Name:\tapi\nThreads:\t27\nVmRSS:\t1024 kB\n";
        assert_eq!(parse_status_usize(status, "Threads"), Some(27));
        assert_eq!(parse_status_usize(status, "Missing"), None);
        assert_eq!(parse_trimmed_usize(" max\n"), None);
    }

    #[cfg(unix)]
    #[test]
    fn pid1_reaper_preserves_exit_and_signal_statuses() {
        assert_eq!(wait_status_exit_code(7 << 8), 7);
        assert_eq!(wait_status_exit_code(libc::SIGKILL), 128 + libc::SIGKILL);
    }

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
    fn streaming_stdout_is_consumed_without_process_owned_buffering() {
        let mut command = Command::new("sh");
        command.arg("-c").arg("cat; printf diagnostic >&2");

        let output = run_with_stdout(
            &mut command,
            Some(b"streamed input".to_vec()),
            ProcessLimits::new(Duration::from_secs(1)),
            "streaming output",
            |mut stdout| {
                let mut retained = Vec::new();
                stdout.read_to_end(&mut retained)?;
                Ok::<_, std::io::Error>(retained)
            },
        )
        .unwrap();

        assert!(output.status.success());
        assert_eq!(output.value, b"streamed input");
        assert_eq!(output.stderr, b"diagnostic");
    }

    #[test]
    fn stdin_reader_is_copied_incrementally() {
        let mut command = Command::new("sh");
        command.arg("-c").arg("wc -c");

        let output = run_with_stdin_reader(
            &mut command,
            std::io::repeat(b'x').take(3 * 1024 * 1024),
            ProcessLimits::new(Duration::from_secs(2)),
            "streaming input",
        )
        .unwrap();

        assert!(output.status.success());
        assert_eq!(String::from_utf8(output.stdout).unwrap().trim(), "3145728");
    }

    #[test]
    fn streaming_stdout_timeout_kills_the_child() {
        let mut command = Command::new("sh");
        command.arg("-c").arg("printf ready; sleep 5");
        let started_at = Instant::now();

        let error = run_with_stdout(
            &mut command,
            None,
            ProcessLimits::new(Duration::from_millis(50)),
            "slow stream",
            |mut stdout| {
                std::io::copy(&mut stdout, &mut std::io::sink())?;
                Ok::<_, std::io::Error>(())
            },
        )
        .unwrap_err();

        assert!(matches!(
            error,
            StreamingProcessError::Process(ProcessError::TimedOut { .. })
        ));
        assert!(started_at.elapsed() < Duration::from_secs(2));
    }

    #[test]
    fn streaming_stdout_consumer_failure_kills_the_child() {
        let mut command = Command::new("sh");
        command.arg("-c").arg("printf ready; sleep 5");
        let started_at = Instant::now();

        let error = run_with_stdout(
            &mut command,
            None,
            ProcessLimits::new(Duration::from_secs(10)),
            "rejected stream",
            |_stdout| Err::<(), _>("disk full"),
        )
        .unwrap_err();

        assert!(matches!(
            error,
            StreamingProcessError::Consumer("disk full")
        ));
        assert!(started_at.elapsed() < Duration::from_secs(2));
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
