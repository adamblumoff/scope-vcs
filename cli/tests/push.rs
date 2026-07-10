mod support;

use std::{fs, process::Command};
use support::*;

#[test]
fn push_help_exposes_remote_and_no_review_options() {
    let output = Command::new(env!("CARGO_BIN_EXE_scope"))
        .args(["push", "--help"])
        .output()
        .unwrap();

    assert_success(&output, "scope push --help");
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("--remote <REMOTE>"), "{stdout}");
    assert!(stdout.contains("--no-review"), "{stdout}");
    assert!(!stdout.contains("--yes"), "{stdout}");
    assert!(!stdout.contains("-y"), "{stdout}");
}

#[test]
fn push_refuses_non_git_directory_before_login() {
    let dir = TempDir::new("non-git");
    let output = scope_command(dir.path()).args(["push"]).output().unwrap();

    assert_failure(&output, "scope push outside git repo");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("run scope push from inside an existing Git repository"),
        "{stderr}"
    );
    assert!(!stderr.contains("start browser login"), "{stderr}");
}

#[test]
fn push_refuses_git_repo_without_head_before_login() {
    let dir = TempDir::new("no-head");
    run_git(dir.path(), ["-c", "init.defaultBranch=main", "init"]);

    let output = scope_command(dir.path()).args(["push"]).output().unwrap();

    assert_failure(&output, "scope push without HEAD");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("create at least one Git commit before running scope push"),
        "{stderr}"
    );
    assert!(!stderr.contains("start browser login"), "{stderr}");
}

#[test]
fn push_creates_missing_config_before_remote_lookup() {
    let dir = TempDir::new("missing-config");
    create_repo_with_readme(dir.path());

    let output = scope_command(dir.path())
        .args(["push", "--no-review"])
        .output()
        .unwrap();

    assert_failure(&output, "scope push without remote");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("Scope remote 'scope' is not configured. Run: scope init"),
        "{stderr}"
    );
    assert!(dir.path().join(".scope/repo.json").is_file());
    assert!(
        stderr.contains("Working tree has uncommitted changes."),
        "{stderr}"
    );
    assert!(!stderr.contains("start browser login"), "{stderr}");
}

#[test]
fn push_refuses_invalid_config_before_remote_lookup() {
    let dir = TempDir::new("invalid-config");
    create_repo_with_readme(dir.path());
    fs::create_dir_all(dir.path().join(".scope")).unwrap();
    fs::write(
        dir.path().join(".scope/repo.json"),
        r#"{
  "kind": "wrong",
  "version": 1,
  "visibility": {
    "default": "private",
    "rules": []
  }
}
"#,
    )
    .unwrap();
    run_git(dir.path(), ["add", ".scope/repo.json"]);
    commit_all(dir.path(), "add invalid config");

    let output = scope_command(dir.path())
        .args(["push", "--no-review"])
        .output()
        .unwrap();

    assert_failure(&output, "scope push with invalid config");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("repo config kind must be scope.repo-config"),
        "{stderr}"
    );
    assert!(
        !stderr.contains("Scope remote 'scope' is not configured"),
        "{stderr}"
    );
    assert!(!stderr.contains("start browser login"), "{stderr}");
}

#[test]
fn push_allows_uncommitted_config_before_remote_lookup() {
    let dir = TempDir::new("uncommitted-config");
    create_repo_with_readme(dir.path());
    write_valid_config(dir.path());

    let output = scope_command(dir.path())
        .args(["push", "--no-review"])
        .output()
        .unwrap();

    assert_failure(&output, "scope push without remote");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("Scope remote 'scope' is not configured. Run: scope init"),
        "{stderr}"
    );
    assert!(
        stderr.contains("Working tree has uncommitted changes."),
        "{stderr}"
    );
    assert!(!stderr.contains("start browser login"), "{stderr}");
}

#[test]
fn push_warns_on_dirty_working_tree_before_remote_lookup_failure() {
    let dir = TempDir::new("dirty");
    create_repo_with_head_and_config(dir.path());
    fs::write(dir.path().join("dirty.txt"), "uncommitted\n").unwrap();

    let output = scope_command(dir.path())
        .args(["push", "--no-review"])
        .output()
        .unwrap();

    assert_failure(&output, "scope push without remote");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("Working tree has uncommitted changes."),
        "{stderr}"
    );
    assert!(
        stderr.contains("Only committed HEAD will be pushed to Scope."),
        "{stderr}"
    );
    assert!(
        stderr.contains("Scope remote 'scope' is not configured. Run: scope init"),
        "{stderr}"
    );
    assert!(!stderr.contains("start browser login"), "{stderr}");
}

#[test]
fn push_requires_review_tty_before_remote_lookup() {
    let dir = TempDir::new("review-non-tty");
    create_repo_with_head_and_config(dir.path());

    let output = scope_command(dir.path()).args(["push"]).output().unwrap();

    assert_failure(&output, "scope push without review tty");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("scope push review requires an interactive terminal"),
        "{stderr}"
    );
    assert!(
        !stderr.contains("Scope remote 'scope' is not configured"),
        "{stderr}"
    );
    assert!(!stderr.contains("start browser login"), "{stderr}");
}

fn write_valid_config(cwd: &std::path::Path) {
    fs::create_dir_all(cwd.join(".scope")).unwrap();
    fs::write(
        cwd.join(".scope/repo.json"),
        r#"{
  "kind": "scope.repo-config",
  "version": 1,
  "visibility": {
    "default": "private",
    "rules": []
  },
  "history": {
    "rewrites": []
  }
}
"#,
    )
    .unwrap();
}

fn create_repo_with_head_and_config(cwd: &std::path::Path) {
    create_repo_with_head(cwd);
    write_valid_config(cwd);
}
