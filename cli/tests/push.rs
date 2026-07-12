mod support;

use std::fs;
use support::*;

#[test]
fn push_stops_at_repository_preconditions_before_login() {
    let non_git = TempDir::new("non-git");
    scope_failure(
        non_git.path(),
        ["push"],
        "run scope push from inside an existing Git repository",
    );

    let no_head = TempDir::new("no-head");
    run_git(no_head.path(), ["-c", "init.defaultBranch=main", "init"]);
    scope_failure(
        no_head.path(),
        ["push"],
        "create at least one Git commit before running scope push",
    );
}

#[test]
fn push_creates_missing_config_before_remote_lookup() {
    let dir = TempDir::new("missing-config");
    create_repo_with_head(dir.path());
    let stderr = scope_failure(
        dir.path(),
        ["push", "--no-review"],
        "no Scope Git remote found; pass --remote <name> or run scope init",
    );
    assert!(dir.path().join(".scope/repo.json").is_file());
    assert!(stderr.contains("Working tree has uncommitted changes."));
}

#[test]
fn push_validates_config_before_remote_lookup() {
    let dir = TempDir::new("invalid-config");
    create_repo_with_head(dir.path());
    fs::create_dir_all(dir.path().join(".scope")).unwrap();
    fs::write(
        dir.path().join(".scope/repo.json"),
        r#"{
      "kind": "wrong", "version": 1,
      "visibility": { "default": "private", "rules": [] }
    }"#,
    )
    .unwrap();
    run_git(dir.path(), ["add", ".scope/repo.json"]);
    commit_all(dir.path(), "add invalid config");

    let stderr = scope_failure(
        dir.path(),
        ["push", "--no-review"],
        "repo config kind must be scope.repo-config",
    );
    assert!(!stderr.contains("Scope remote 'scope' is not configured"));
}

#[test]
fn push_warns_about_dirty_state_before_remote_lookup() {
    let dir = configured_repo("dirty");
    fs::write(dir.path().join("dirty.txt"), "uncommitted\n").unwrap();
    let stderr = scope_failure(
        dir.path(),
        ["push", "--no-review"],
        "no Scope Git remote found; pass --remote <name> or run scope init",
    );
    assert!(stderr.contains("Working tree has uncommitted changes."));
    assert!(stderr.contains("Only committed HEAD will be pushed to Scope."));
}

#[test]
fn push_requires_review_tty_before_remote_lookup() {
    let dir = configured_repo("review-non-tty");
    let stderr = scope_failure(
        dir.path(),
        ["push"],
        "scope push review requires an interactive terminal",
    );
    assert!(!stderr.contains("Scope remote 'scope' is not configured"));
}

fn configured_repo(label: &str) -> TempDir {
    let dir = TempDir::new(label);
    create_repo_with_head(dir.path());
    fs::create_dir_all(dir.path().join(".scope")).unwrap();
    fs::write(
        dir.path().join(".scope/repo.json"),
        r#"{
      "kind": "scope.repo-config", "version": 1,
      "visibility": { "default": "private", "rules": [] },
      "history": { "rewrites": [] }
    }"#,
    )
    .unwrap();
    dir
}
