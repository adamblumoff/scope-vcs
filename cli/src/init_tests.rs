use super::*;
use crate::test_support::TestDir;
use std::{path::Path, process::Command};

const OLD_REMOTE: &str = "https://old.scope.example/git/adam/sample";
const NEW_REMOTE: &str = "https://scope.example/git/adam/sample";

#[test]
fn configure_remote_adds_or_updates_the_remote_idempotently() {
    let new_dir = TestDir::git_repo("init-add-remote", "main");
    let init = repo_init("scope", NEW_REMOTE);

    configure_remote(new_dir.path(), &init).unwrap();
    configure_remote(new_dir.path(), &init).unwrap();

    assert_eq!(
        git_config(new_dir.path(), "remote.scope.url"),
        vec![NEW_REMOTE]
    );
    assert_eq!(
        git_config(new_dir.path(), "remote.scope.fetch"),
        vec!["+refs/heads/*:refs/remotes/scope/*"]
    );
    assert!(git_config(new_dir.path(), "remote.scope.pushurl").is_empty());
    assert_eq!(
        git_config(new_dir.path(), &format!("credential.{NEW_REMOTE}.helper")),
        vec!["", "!scope git-credential"]
    );

    let existing_dir = TestDir::git_repo("init-update-remote", "main");
    commit_empty(&existing_dir);
    existing_dir.run_git(["remote", "add", "scope", OLD_REMOTE]);
    existing_dir.run_git(["remote", "set-url", "--push", "scope", OLD_REMOTE]);
    existing_dir.run_git(["config", "--local", "remote.scope.mirror", "true"]);
    existing_dir.run_git([
        "config",
        "--local",
        "remote.scope.uploadpack",
        "custom-upload",
    ]);
    existing_dir.run_git(["update-ref", "refs/remotes/scope/legacy", "HEAD"]);

    configure_remote(existing_dir.path(), &init).unwrap();

    assert_eq!(
        git_config(existing_dir.path(), "remote.scope.url"),
        vec![NEW_REMOTE]
    );
    assert!(git_config(existing_dir.path(), "remote.scope.pushurl").is_empty());
    assert_eq!(git_remote_url(existing_dir.path(), false), NEW_REMOTE);
    assert_eq!(git_remote_url(existing_dir.path(), true), NEW_REMOTE);
    assert!(git_config(existing_dir.path(), "remote.scope.mirror").is_empty());
    assert!(git_config(existing_dir.path(), "remote.scope.uploadpack").is_empty());
    assert!(git_ref_exists(
        existing_dir.path(),
        "refs/remotes/scope/legacy"
    ));
}

#[test]
fn remote_snapshot_restores_existing_and_absent_remote_state() {
    let init = repo_init("scope", NEW_REMOTE);
    let existing_dir = TestDir::git_repo("init-restore-remote", "main");
    commit_empty(&existing_dir);
    existing_dir.run_git(["remote", "add", "scope", OLD_REMOTE]);
    existing_dir.run_git(["remote", "set-url", "--push", "scope", OLD_REMOTE]);
    existing_dir.run_git(["config", "--local", "remote.scope.mirror", "true"]);
    existing_dir.run_git([
        "config",
        "--local",
        "remote.scope.uploadpack",
        "custom-upload",
    ]);
    existing_dir.run_git(["update-ref", "refs/remotes/scope/legacy", "HEAD"]);
    existing_dir.run_git([
        "config",
        "--local",
        "--add",
        &format!("credential.{NEW_REMOTE}.helper"),
        "prior-helper",
    ]);
    let before = config_snapshot(existing_dir.path(), &init);
    let snapshot = RemoteConfigSnapshot::capture(existing_dir.path(), &init).unwrap();

    configure_remote(existing_dir.path(), &init).unwrap();
    snapshot.restore(existing_dir.path()).unwrap();

    assert_eq!(config_snapshot(existing_dir.path(), &init), before);
    assert_eq!(git_remote_url(existing_dir.path(), false), OLD_REMOTE);
    assert_eq!(git_remote_url(existing_dir.path(), true), OLD_REMOTE);
    assert!(git_ref_exists(
        existing_dir.path(),
        "refs/remotes/scope/legacy"
    ));

    let absent_dir = TestDir::git_repo("init-restore-absent-remote", "main");
    let snapshot = RemoteConfigSnapshot::capture(absent_dir.path(), &init).unwrap();
    configure_remote(absent_dir.path(), &init).unwrap();
    snapshot.restore(absent_dir.path()).unwrap();

    assert!(git_remotes(absent_dir.path()).is_empty());
    assert!(
        config_snapshot(absent_dir.path(), &init)
            .into_iter()
            .all(|(_, values)| values.is_empty())
    );
}

fn repo_init(remote_name: &str, git_remote_url: &str) -> RepoInitResponse {
    serde_json::from_value(serde_json::json!({
        "repo": {
            "id": "repo_test",
            "owner_handle": "adam",
            "name": "sample",
            "lifecycle_state": "AwaitingFirstPush",
            "change_version": 1,
            "access": {
                "actor": "Owner",
                "can_read_private_files": true,
                "can_push": true,
                "can_change_file_visibility": true,
                "can_apply_changes": true,
                "can_manage_members": true,
                "can_delete_repo": true
            },
            "open_request_count": 0,
            "request_permissions": { "can_start_request": true }
        },
        "git_remote_url": git_remote_url,
        "remote_name": remote_name,
        "push_branch": "main",
        "token": null,
        "push_token": null
    }))
    .unwrap()
}

fn config_snapshot(path: &Path, init: &RepoInitResponse) -> Vec<(String, Vec<String>)> {
    remote_config_keys(path, init)
        .unwrap()
        .into_iter()
        .map(|key| {
            let values = git_config(path, &key);
            (key, values)
        })
        .collect()
}

fn git_config(path: &Path, key: &str) -> Vec<String> {
    local_config_values(path, key).unwrap()
}

fn git_remote_url(path: &Path, push: bool) -> String {
    let mut args = vec!["remote", "get-url"];
    if push {
        args.push("--push");
    }
    args.push("scope");
    let output = Command::new("git")
        .current_dir(path)
        .args(args)
        .output()
        .unwrap();
    assert!(output.status.success());
    String::from_utf8(output.stdout).unwrap().trim().to_string()
}

fn git_remotes(path: &Path) -> Vec<String> {
    let output = Command::new("git")
        .current_dir(path)
        .arg("remote")
        .output()
        .unwrap();
    assert!(output.status.success());
    String::from_utf8(output.stdout)
        .unwrap()
        .lines()
        .map(str::to_string)
        .collect()
}

fn git_ref_exists(path: &Path, name: &str) -> bool {
    Command::new("git")
        .current_dir(path)
        .args(["show-ref", "--verify", "--quiet", name])
        .status()
        .unwrap()
        .success()
}

fn commit_empty(dir: &TestDir) {
    dir.run_git([
        "-c",
        "user.name=Scope Test",
        "-c",
        "user.email=scope@example.test",
        "commit",
        "--quiet",
        "--allow-empty",
        "-m",
        "fixture",
    ]);
}
