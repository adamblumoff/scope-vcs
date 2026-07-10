mod support;

use std::{fs, process::Command};
use support::*;

#[test]
fn init_help_exposes_name_and_omits_visibility_flags() {
    let output = Command::new(env!("CARGO_BIN_EXE_scope"))
        .args(["init", "--help"])
        .output()
        .unwrap();

    assert_success(&output, "scope init --help");
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("--name <NAME>"), "{stdout}");
    assert!(!stdout.contains("--public"), "{stdout}");
    assert!(!stdout.contains("--private"), "{stdout}");
    assert!(!stdout.contains("[NAME]"), "{stdout}");
}

#[test]
fn init_refuses_non_git_directory_without_creating_repo() {
    let dir = TempDir::new("non-git");
    let output = scope_command(dir.path())
        .args(["init", "--name", "sample"])
        .output()
        .unwrap();

    assert_failure(&output, "scope init outside git repo");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("run scope init from inside an existing Git repository"),
        "{stderr}"
    );
    assert!(!dir.path().join(".git").exists());
}

#[test]
fn init_refuses_git_repo_without_head() {
    let dir = TempDir::new("no-head");
    run_git(dir.path(), ["-c", "init.defaultBranch=main", "init"]);

    let output = scope_command(dir.path())
        .args(["init", "--name", "sample"])
        .output()
        .unwrap();

    assert_failure(&output, "scope init without HEAD");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("create at least one Git commit before running scope init"),
        "{stderr}"
    );
}

#[test]
fn init_without_visibility_flags_continues_to_auth() {
    let dir = TempDir::new("no-visibility-prompt");
    create_repo_with_head(dir.path());

    let output = scope_command(dir.path())
        .args(["init", "--name", "sample"])
        .output()
        .unwrap();

    assert_failure(&output, "scope init without API");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(!stderr.contains("Default visibility"), "{stderr}");
    assert!(stderr.contains("start browser login"), "{stderr}");
}

#[test]
fn init_warns_on_dirty_working_tree_and_continues_to_auth() {
    let dir = TempDir::new("dirty");
    create_repo_with_head(dir.path());
    fs::write(dir.path().join("dirty.txt"), "uncommitted\n").unwrap();

    let output = scope_command(dir.path())
        .args(["init", "--name", "sample"])
        .output()
        .unwrap();

    assert_failure(&output, "scope init with dirty working tree");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("Working tree has uncommitted changes."),
        "{stderr}"
    );
    assert!(
        stderr.contains("Only committed HEAD will be pushed to Scope."),
        "{stderr}"
    );
    assert!(stderr.contains("start browser login"), "{stderr}");
}
