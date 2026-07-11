mod support;

use std::fs;
use support::*;

#[test]
fn request_sync_refuses_dirty_worktree_before_login() {
    let dir = TempDir::new("dirty");
    create_repo_with_head(dir.path());
    fs::write(dir.path().join("dirty.txt"), "uncommitted\n").unwrap();

    scope_failure(
        dir.path(),
        ["request", "sync-main"],
        "commit or stash local changes before running scope request sync-main",
    );
}

#[test]
fn request_sync_refuses_unattached_branch_before_login() {
    let dir = TempDir::new("sync-unattached");
    create_repo_with_head(dir.path());

    scope_failure(
        dir.path(),
        ["request", "sync-main"],
        "scope request sync-main requires a Scope request branch",
    );
}

#[test]
fn request_submit_refuses_unattached_branch_before_login() {
    let dir = TempDir::new("submit-unattached");
    create_repo_with_head(dir.path());

    scope_failure(
        dir.path(),
        ["request", "submit", "--stake-credits", "1"],
        "scope request submit requires a Scope request branch",
    );
}

#[test]
fn request_submit_refuses_detached_head_before_login() {
    let dir = TempDir::new("detached");
    create_repo_with_head(dir.path());
    run_git(dir.path(), ["checkout", "--detach"]);

    scope_failure(
        dir.path(),
        ["request", "submit", "--stake-credits", "1"],
        "request commands require a named local branch",
    );
}
