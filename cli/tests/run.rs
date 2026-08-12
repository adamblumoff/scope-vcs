mod support;

use support::*;

#[test]
fn run_control_commands_require_an_id_before_repository_or_auth_work() {
    let dir = TempDir::new("run-control-id");
    create_repo_with_head(dir.path());
    for command in ["show", "watch", "cancel", "retry"] {
        scope_failure_with_code(
            dir.path(),
            ["run", command],
            &format!("scope run {command} requires a run ID"),
            2,
        );
    }
}

#[test]
fn run_help_exposes_stable_detail_inspection() {
    let dir = TempDir::new("run-help");
    let output = scope_command(dir.path())
        .args(["run", "--help"])
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(stdout.contains("show/watch/cancel/retry"), "{stdout}");
}

#[test]
fn json_show_is_admitted_as_a_machine_readable_command() {
    let dir = TempDir::new("run-show-json");
    let output = scope_command(dir.path())
        .args(["--json", "run", "show", "run-123"])
        .output()
        .unwrap();

    assert_failure(&output, "run show outside a repository");
    assert!(output.stdout.is_empty());
    assert_eq!(output.status.code(), Some(1));
    let error: scope_api_contract::ErrorResponse = serde_json::from_slice(&output.stderr).unwrap();
    assert_eq!(error.code, scope_api_contract::ErrorCode::Internal);
    assert_eq!(
        error.message,
        "run scope run show from inside an existing Git repository",
    );
}
