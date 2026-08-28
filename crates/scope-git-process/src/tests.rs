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
