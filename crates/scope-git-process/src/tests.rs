use super::*;
#[cfg(unix)]
use crate::lifecycle::wait_status_exit_code;
use crate::lifecycle::{parse_status_usize, parse_trimmed_usize};
use std::{
    io::Read,
    process::Command,
    time::{Duration, Instant},
};

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
        |mut stdout, _cancellation| {
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
        |mut stdout, _cancellation| {
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
        |_stdout, _cancellation| Err::<(), _>("disk full"),
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

#[cfg(unix)]
#[test]
fn streaming_timeout_cancels_downstream_work_and_kills_descendants() {
    let pid_file = tempfile::NamedTempFile::new().unwrap();
    let mut command = Command::new("sh");
    command
        .arg("-c")
        .arg("sleep 30 & descendant=$!; printf '%s' \"$descendant\" > \"$1\"; wait")
        .arg("sh")
        .arg(pid_file.path());
    let started_at = Instant::now();

    let error = run_with_stdout(
        &mut command,
        None,
        ProcessLimits::new(Duration::from_millis(100)),
        "stalled downstream consumer",
        |_stdout, cancellation| {
            tokio::runtime::Builder::new_current_thread()
                .build()
                .unwrap()
                .block_on(cancellation.cancelled());
            assert!(cancellation.is_cancelled());
            Ok::<_, std::io::Error>(())
        },
    )
    .unwrap_err();

    assert!(matches!(
        error,
        StreamingProcessError::Process(ProcessError::TimedOut { .. })
    ));
    assert!(started_at.elapsed() < Duration::from_secs(2));

    let descendant = std::fs::read_to_string(pid_file.path())
        .unwrap()
        .parse::<u32>()
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(1);
    loop {
        let state = process_state(descendant);
        if state.as_deref().is_none_or(|state| state == "Z") {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "descendant {descendant} survived timeout in state {state:?}"
        );
        std::thread::sleep(Duration::from_millis(1));
    }
}

#[cfg(unix)]
fn process_state(pid: u32) -> Option<String> {
    let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    let command_end = stat.rfind(')')?;
    stat.get(command_end + 2..)?
        .split_whitespace()
        .next()
        .map(str::to_string)
}

#[cfg(unix)]
#[test]
fn unwinding_after_spawn_kills_and_reaps_the_owned_process() {
    let mut command = Command::new("sh");
    command.args(["-c", "exec sleep 60"]);
    configure_process_group(&mut command);
    let child = command.spawn().unwrap();
    let pid = child.id() as libc::pid_t;
    let result = std::panic::catch_unwind(move || {
        let _child = crate::lifecycle::ChildGuard::new(child);
        panic!("injected runner setup panic");
    });
    assert!(result.is_err());
    // A killed but unreaped child still has a PID and is waitable.
    assert_eq!(unsafe { libc::kill(pid, 0) }, -1);
    assert_eq!(
        std::io::Error::last_os_error().raw_os_error(),
        Some(libc::ESRCH)
    );
    assert_eq!(
        unsafe { libc::waitpid(pid, std::ptr::null_mut(), libc::WNOHANG) },
        -1
    );
    assert_eq!(
        std::io::Error::last_os_error().raw_os_error(),
        Some(libc::ECHILD)
    );
}
