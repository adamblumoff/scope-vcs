mod support;

use std::process::Output;
use support::*;

#[test]
fn request_start_requires_a_name_before_login() {
    let dir = TempDir::new("start-name");
    create_repo_with_head(dir.path());

    scope_failure(
        dir.path(),
        ["request", "start"],
        "the following required arguments were not provided",
    );
}

#[test]
fn obsolete_request_transport_commands_are_removed() {
    let dir = TempDir::new("removed-request-transport");
    create_repo_with_head(dir.path());

    for command in ["delete", "join", "pull", "share", "sync-main"] {
        scope_failure(
            dir.path(),
            ["request", command],
            &format!("unrecognized subcommand '{command}'"),
        );
    }
}

#[test]
fn obsolete_request_lifecycle_commands_and_aliases_are_removed() {
    let dir = TempDir::new("removed-request-lifecycle");
    create_repo_with_head(dir.path());

    for command in [
        "comment",
        "discuss",
        "needs-response",
        "ready",
        "respond",
        "resolve",
        "working",
    ] {
        scope_failure(
            dir.path(),
            ["request", command],
            &format!("unrecognized subcommand '{command}'"),
        );
    }
}

#[test]
fn request_discussion_body_rules_are_validated_before_login() {
    let dir = TempDir::new("discussion-body");
    create_repo_with_head(dir.path());

    scope_failure(
        dir.path(),
        ["request", "discussion", "start"],
        "the following required arguments were not provided",
    );
    scope_failure(
        dir.path(),
        ["request", "discussion", "reply", "dsc_one"],
        "the following required arguments were not provided",
    );
    scope_failure(
        dir.path(),
        ["request", "discussion", "reopen", "dsc_one"],
        "the following required arguments were not provided",
    );
    scope_failure(
        dir.path(),
        [
            "request",
            "discussion",
            "start",
            "--body",
            "Question",
            "--body-file",
            "question.md",
        ],
        "cannot be used with",
    );
}

#[test]
fn request_discussion_anchor_dependencies_are_validated_before_login() {
    let dir = TempDir::new("discussion-anchor");
    create_repo_with_head(dir.path());

    scope_failure(
        dir.path(),
        [
            "request",
            "discussion",
            "start",
            "--body",
            "Question",
            "--commit",
            "0123456789abcdef",
        ],
        "--revision <REVISION>",
    );
    scope_failure(
        dir.path(),
        [
            "request",
            "discussion",
            "start",
            "--body",
            "Question",
            "--path",
            "src/lib.rs",
        ],
        "--commit <OID>",
    );
}

#[test]
fn request_rate_requires_score_and_reason_before_login() {
    let dir = TempDir::new("rate-request");
    create_repo_with_head(dir.path());

    scope_failure(
        dir.path(),
        ["request", "rate"],
        "the following required arguments were not provided",
    );
    scope_failure(
        dir.path(),
        ["request", "rate", "--score", "6", "--reason", "Excellent"],
        "not in 1..=5",
    );
}

#[test]
fn json_usage_errors_use_the_shared_schema_and_exit_two() {
    let dir = TempDir::new("request-json-usage");
    create_repo_with_head(dir.path());

    let output = scope_command(dir.path())
        .args([
            "--json", "request", "rate", "--score", "6", "--reason", "Invalid",
        ])
        .output()
        .unwrap();

    assert_failure(&output, "request JSON usage error");
    assert!(output.stdout.is_empty());
    assert_eq!(output.status.code(), Some(2));
    let error: scope_api_contract::ErrorResponse = serde_json::from_slice(&output.stderr).unwrap();
    assert_eq!(error.code, scope_api_contract::ErrorCode::BadRequest);
    assert!(!error.retryable);
}

#[test]
fn global_json_rejects_commands_without_typed_results() {
    let dir = TempDir::new("json-command-scope");
    create_repo_with_head(dir.path());

    let output = scope_command(dir.path())
        .args(["--json", "rules", "sync"])
        .output()
        .unwrap();

    assert_failure(&output, "unsupported JSON command");
    assert!(output.stdout.is_empty());
    assert_eq!(output.status.code(), Some(2));
    let error: scope_api_contract::ErrorResponse = serde_json::from_slice(&output.stderr).unwrap();
    assert_eq!(error.code, scope_api_contract::ErrorCode::BadRequest);
    assert_eq!(
        error.message,
        "--json currently supports request commands only"
    );
}

#[test]
fn json_mode_preserves_successful_help_and_version_control_flow() {
    let dir = TempDir::new("json-help");
    create_repo_with_head(dir.path());

    for args in [
        vec!["--json", "--help"],
        vec!["request", "--json", "--help"],
        vec!["--json", "--version"],
    ] {
        let output = scope_command(dir.path()).args(&args).output().unwrap();
        assert!(
            output.status.success(),
            "{args:?}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(!output.stdout.is_empty(), "{args:?}");
        assert!(output.stderr.is_empty(), "{args:?}");
    }
}

#[test]
fn request_submit_reaches_auth_without_extra_arguments() {
    let dir = TempDir::new("submit-request");
    create_repo_with_head(dir.path());

    let output = scope_command(dir.path())
        .args(["request", "submit"])
        .output()
        .unwrap();
    assert_failure(&output, "scope request submit");
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(!stderr.contains("required arguments"), "{stderr}");
    assert!(stderr.contains("start browser login"), "{stderr}");
}

#[test]
fn every_request_command_accepts_the_global_json_mode_and_returns_json_failures() {
    let dir = TempDir::new("request-json");
    create_repo_with_head(dir.path());

    let commands = [
        vec!["--json", "request", "start", "change"],
        vec!["request", "push", "--json"],
        vec!["--json", "request", "submit", "--yes"],
        vec!["request", "edit", "--title", "Updated", "--json"],
        vec!["--json", "request", "invite", "river"],
        vec!["request", "uninvite", "river", "--json"],
        vec!["--json", "request", "leave"],
        vec!["request", "merge", "--yes", "--json"],
        vec![
            "--json", "request", "rate", "--score", "5", "--reason", "Clear",
        ],
        vec![
            "request",
            "discussion",
            "start",
            "--body",
            "Question",
            "--json",
        ],
        vec![
            "request",
            "discussion",
            "reply",
            "dsc_one",
            "--body",
            "Answer",
            "--json",
        ],
        vec!["request", "discussion", "resolve", "dsc_one", "--json"],
        vec![
            "request",
            "discussion",
            "reopen",
            "dsc_one",
            "--body",
            "New evidence",
            "--json",
        ],
        vec!["--json", "request", "show"],
        vec!["request", "list", "--json"],
        vec!["--json", "request", "status"],
        vec!["request", "close", "--yes", "--json"],
    ];
    for args in commands {
        let output = scope_command(dir.path()).args(&args).output().unwrap();
        assert_failure(&output, "request JSON output");
        assert!(
            output.stdout.is_empty(),
            "{}",
            String::from_utf8_lossy(&output.stdout)
        );
        let stderr = String::from_utf8(output.stderr).unwrap();
        let error: scope_api_contract::ErrorResponse = serde_json::from_str(stderr.trim()).unwrap();
        assert_eq!(
            error.code,
            scope_api_contract::ErrorCode::Unauthorized,
            "{args:?}: {stderr}"
        );
        assert!(!error.retryable);
        assert_eq!(output.status.code(), Some(3));
    }
}

#[test]
fn request_edit_requires_a_title_or_description_file_before_login() {
    let dir = TempDir::new("edit-content");
    create_repo_with_head(dir.path());

    scope_failure(
        dir.path(),
        ["request", "edit"],
        "the following required arguments were not provided",
    );
}

#[test]
fn request_help_exposes_the_complete_approved_vocabulary() {
    let dir = TempDir::new("request-help");
    create_repo_with_head(dir.path());

    let output = scope_command(dir.path())
        .args(["request", "--help"])
        .output()
        .unwrap();
    assert_success(&output, "scope request --help");
    let stdout = String::from_utf8(output.stdout).unwrap();

    for command in [
        "close",
        "discussion",
        "edit",
        "invite",
        "leave",
        "list",
        "merge",
        "push",
        "show",
        "start",
        "status",
        "submit",
        "uninvite",
    ] {
        assert!(
            stdout.lines().any(|line| {
                line.trim_start()
                    .strip_prefix(command)
                    .is_some_and(|rest| rest.starts_with(char::is_whitespace))
            }),
            "missing {command:?} from help:\n{stdout}"
        );
    }
}

#[test]
fn request_command_help_uses_the_shared_target_flags() {
    let dir = TempDir::new("request-target-help");
    create_repo_with_head(dir.path());

    for command in [
        "close", "edit", "invite", "leave", "merge", "push", "show", "status", "submit", "uninvite",
    ] {
        let output = scope_command(dir.path())
            .args(["request", command, "--help"])
            .output()
            .unwrap();
        assert_success(&output, &format!("scope request {command} --help"));
        let stdout = String::from_utf8(output.stdout).unwrap();
        assert!(stdout.contains("--remote <REMOTE>"), "{command}:\n{stdout}");
        assert!(
            stdout.contains("--request <REQUEST>"),
            "{command}:\n{stdout}"
        );
    }

    for command in ["start", "reply", "resolve", "reopen"] {
        let output = scope_command(dir.path())
            .args(["request", "discussion", command, "--help"])
            .output()
            .unwrap();
        assert_success(
            &output,
            &format!("scope request discussion {command} --help"),
        );
        let stdout = String::from_utf8(output.stdout).unwrap();
        assert!(stdout.contains("--remote <REMOTE>"), "{command}:\n{stdout}");
        assert!(
            stdout.contains("--request <REQUEST>"),
            "{command}:\n{stdout}"
        );
    }
}

fn assert_success(output: &Output, action: &str) {
    assert!(
        output.status.success(),
        "{action} failed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}
