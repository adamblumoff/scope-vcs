mod support;

use std::process::Command;
use support::*;

#[test]
fn review_help_is_available() {
    let output = Command::new(env!("CARGO_BIN_EXE_scope"))
        .args(["review", "--help"])
        .output()
        .unwrap();

    assert_success(&output, "scope review --help");
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("Usage: scope review"), "{stdout}");
}

#[test]
fn review_refuses_non_git_directory() {
    let dir = TempDir::new("non-git");
    let output = scope_command(dir.path()).args(["review"]).output().unwrap();

    assert_failure(&output, "scope review outside git repo");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("run scope review from inside an existing Git repository"),
        "{stderr}"
    );
}

#[test]
fn review_requires_interactive_terminal_before_creating_config() {
    let dir = TempDir::new("non-tty");
    create_repo_with_head(dir.path());

    let output = scope_command(dir.path()).args(["review"]).output().unwrap();

    assert_failure(&output, "scope review without tty");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("scope review requires an interactive terminal"),
        "{stderr}"
    );
    assert!(!dir.path().join(".scope/repo.json").exists());
}

#[test]
fn review_can_start_before_first_commit() {
    let dir = TempDir::new("no-head");
    run_git(dir.path(), ["-c", "init.defaultBranch=main", "init"]);

    let output = scope_command(dir.path()).args(["review"]).output().unwrap();

    assert_failure(&output, "scope review without HEAD");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("scope review requires an interactive terminal"),
        "{stderr}"
    );
    assert!(
        !stderr.contains("create at least one Git commit before running scope review"),
        "{stderr}"
    );
}
