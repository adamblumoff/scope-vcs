use std::{
    collections::BTreeSet,
    fs,
    io::ErrorKind,
    path::Path,
    process::{Child, Command},
    sync::atomic::{AtomicI32, Ordering},
    thread,
    time::{Duration, Instant},
};

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
pub(crate) fn wait_status_exit_code(status: i32) -> i32 {
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

pub(crate) fn parse_status_usize(contents: &str, name: &str) -> Option<usize> {
    contents.lines().find_map(|line| {
        let (key, value) = line.split_once(':')?;
        (key == name).then(|| parse_trimmed_usize(value)).flatten()
    })
}

pub(crate) fn parse_trimmed_usize(value: &str) -> Option<usize> {
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
pub(crate) fn terminate_and_reap(child: &mut Child) {
    kill_process_group(child.id());
    let _ = child.kill();
    let _ = child.wait();
}

#[cfg(not(unix))]
pub(crate) fn terminate_and_reap(child: &mut Child) {
    let _ = child.kill();
    let _ = child.wait();
}
