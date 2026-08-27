use scope_git_process::{ProcessLimits, run};
use std::{process::Command, time::Duration};

#[test]
fn stderr_is_drained_after_diagnostic_cap() {
    let mut command = Command::new("sh");
    command
        .arg("-c")
        .arg("set -e; dd if=/dev/zero bs=1024 count=20 >&2 2>/dev/null; printf ok");

    let output = run(
        &mut command,
        None,
        ProcessLimits::new(Duration::from_secs(2)),
        "large stderr",
    )
    .unwrap();

    assert_eq!(output.stdout, b"ok");
}
