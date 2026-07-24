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

    for command in ["comment", "needs-response", "respond", "resolve", "submit"] {
        scope_failure(
            dir.path(),
            ["request", command],
            &format!("unrecognized subcommand '{command}'"),
        );
    }
}

#[test]
fn request_discuss_requires_a_body_before_login() {
    let dir = TempDir::new("discuss-body");
    create_repo_with_head(dir.path());

    scope_failure(
        dir.path(),
        ["request", "discuss"],
        "the following required arguments were not provided",
    );
}

#[test]
fn request_ready_requires_a_stake_before_login() {
    let dir = TempDir::new("ready-stake");
    create_repo_with_head(dir.path());

    scope_failure(
        dir.path(),
        ["request", "ready"],
        "the following required arguments were not provided",
    );
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
fn rejected_assessment_requires_a_message_before_login() {
    let dir = TempDir::new("assess-rejected-message");
    create_repo_with_head(dir.path());

    scope_failure(
        dir.path(),
        ["request", "assess", "rejected"],
        "the following required arguments were not provided",
    );
}

#[test]
fn request_assess_rejects_obsolete_dispositions_before_login() {
    let dir = TempDir::new("assess-outcome");
    create_repo_with_head(dir.path());

    for outcome in [
        "changes-requested",
        "duplicate",
        "needs-response",
        "resolved",
    ] {
        scope_failure(dir.path(), ["request", "assess", outcome], "invalid value");
    }
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
        "assess",
        "close",
        "discuss",
        "edit",
        "hold",
        "invite",
        "leave",
        "list",
        "merge",
        "push",
        "ready",
        "request-changes",
        "show",
        "start",
        "status",
        "unhold",
        "uninvite",
        "working",
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
        "assess",
        "close",
        "discuss",
        "edit",
        "hold",
        "invite",
        "leave",
        "merge",
        "push",
        "ready",
        "request-changes",
        "show",
        "status",
        "unhold",
        "uninvite",
        "working",
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
}

fn assert_success(output: &Output, action: &str) {
    assert!(
        output.status.success(),
        "{action} failed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}
