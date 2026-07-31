mod support;

use support::*;

#[test]
fn run_control_commands_require_an_id_before_repository_or_auth_work() {
    let dir = TempDir::new("run-control-id");
    create_repo_with_head(dir.path());
    for command in ["watch", "cancel", "retry"] {
        scope_failure(
            dir.path(),
            ["run", command],
            &format!("scope run {command} requires a run ID"),
        );
    }
}

#[test]
fn runner_selection_is_only_valid_when_starting_a_workflow() {
    let dir = TempDir::new("run-control-runner");
    scope_failure(
        dir.path(),
        ["run", "retry", "run_123", "--runner", "linux-box"],
        "--runner is only valid when starting a workflow",
    );
}

#[test]
fn runner_help_exposes_one_time_install_and_ongoing_management() {
    let dir = TempDir::new("runner-help");
    let output = scope_command(dir.path())
        .args(["runner", "--help"])
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");
    let stdout = String::from_utf8(output.stdout).unwrap();
    for command in [
        "install",
        "status",
        "doctor",
        "cache",
        "add-repo",
        "remove-repo",
    ] {
        assert!(stdout.contains(command), "{stdout}");
    }
    assert!(!stdout.contains("daemon"), "{stdout}");
}

#[test]
fn runner_cache_help_exposes_safe_operator_actions() {
    let dir = TempDir::new("runner-cache-help");
    let output = scope_command(dir.path())
        .args(["runner", "cache", "--help"])
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(stdout.contains("list"), "{stdout}");
    assert!(stdout.contains("prune"), "{stdout}");
}
