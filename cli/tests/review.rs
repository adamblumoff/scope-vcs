mod support;

use support::*;

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
