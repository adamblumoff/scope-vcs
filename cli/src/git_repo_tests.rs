use super::*;
use crate::test_support::TestDir;
use std::{fs, path::Path, process::Command};

#[test]
fn git_clone_auth_plan_keeps_bearer_token_out_of_process_args() {
    let plan = git_clone_auth_plan(
        "https://scope.example/git/permissioned/adam/random",
        "scope_cli_secret",
        Some(Path::new("local-dir")),
        Some(2),
    );

    assert_eq!(
        plan.args,
        vec![
            "clone",
            "https://scope.example/git/permissioned/adam/random",
            "local-dir",
        ]
    );
    assert!(!plan.args.iter().any(|arg| arg.contains("scope_cli_secret")));
    assert_eq!(
        plan.env,
        vec![
            ("GIT_CONFIG_COUNT".to_string(), "3".to_string()),
            (
                "GIT_CONFIG_KEY_2".to_string(),
                "http.https://scope.example/git/permissioned/adam/random.extraHeader".to_string()
            ),
            (
                "GIT_CONFIG_VALUE_2".to_string(),
                "Authorization: Bearer scope_cli_secret".to_string()
            ),
        ]
    );
}

#[test]
fn install_scope_fetch_auth_writes_secret_free_credential_helper_for_permissioned_remote() {
    let dir = TestDir::git_repo("scope-fetch-auth", "main");
    let root = dir.path();
    let remote_url = "https://scope.example/git/permissioned/adam/random";

    install_scope_fetch_auth(root, remote_url).unwrap();
    install_scope_fetch_auth(root, remote_url).unwrap();

    let helpers = Command::new("git")
        .current_dir(root)
        .args([
            "config",
            "--local",
            "--get-all",
            &format!("credential.{remote_url}.helper"),
        ])
        .output()
        .unwrap();
    assert!(helpers.status.success());
    assert_eq!(
        String::from_utf8_lossy(&helpers.stdout)
            .lines()
            .collect::<Vec<_>>(),
        vec!["", SCOPE_GIT_CREDENTIAL_HELPER]
    );

    let helper = Command::new("git")
        .current_dir(root)
        .args([
            "config",
            "--local",
            "--get-urlmatch",
            "credential.helper",
            remote_url,
        ])
        .output()
        .unwrap();
    assert!(helper.status.success());
    assert_eq!(
        String::from_utf8_lossy(&helper.stdout).trim(),
        SCOPE_GIT_CREDENTIAL_HELPER
    );
    let use_http_path = Command::new("git")
        .current_dir(root)
        .args([
            "config",
            "--local",
            "--get-urlmatch",
            "credential.useHttpPath",
            remote_url,
        ])
        .output()
        .unwrap();
    assert!(use_http_path.status.success());
    assert_eq!(
        String::from_utf8_lossy(&use_http_path.stdout).trim(),
        "true"
    );
    let config = fs::read_to_string(root.join(".git/config")).unwrap();
    assert!(
        !config.contains("scope_cli_secret"),
        "repo config must not persist Scope session tokens"
    );
}

#[test]
fn install_scope_fetch_auth_rejects_config_injection() {
    let dir = TestDir::git_repo("scope-fetch-auth-injection", "main");
    let root = dir.path();
    assert!(
        install_scope_fetch_auth(
            root,
            "https://scope.example/git/permissioned/adam/random\n[alias]",
        )
        .is_err()
    );
}

#[test]
fn git_push_auth_plan_keeps_bearer_token_out_of_process_args() {
    let plan = git_push_auth_plan(
        "https://scope.example/git/permissioned/adam/random",
        "1234567890123456789012345678901234567890",
        "main",
        "scope_cli_secret",
        "scope_pi_secret",
        Some(2),
    );

    assert_eq!(
        plan.args,
        vec![
            "-c",
            "push.recurseSubmodules=no",
            "push",
            "https://scope.example/git/permissioned/adam/random",
            "1234567890123456789012345678901234567890:refs/heads/main"
        ]
    );
    assert!(!plan.args.iter().any(|arg| arg.contains("scope_cli_secret")));
    assert_eq!(
        plan.env,
        vec![
            ("GIT_CONFIG_COUNT".to_string(), "4".to_string()),
            (
                "GIT_CONFIG_KEY_2".to_string(),
                "http.https://scope.example/git/permissioned/adam/random.extraHeader".to_string()
            ),
            (
                "GIT_CONFIG_VALUE_2".to_string(),
                "Authorization: Bearer scope_cli_secret".to_string()
            ),
            (
                "GIT_CONFIG_KEY_3".to_string(),
                "http.https://scope.example/git/permissioned/adam/random.extraHeader".to_string()
            ),
            (
                "GIT_CONFIG_VALUE_3".to_string(),
                "X-Scope-Push-Intent: scope_pi_secret".to_string()
            ),
        ]
    );
}

#[test]
fn git_fetch_auth_plan_keeps_bearer_token_out_of_process_args() {
    let plan = git_fetch_auth_plan(
        "https://scope.example/git/permissioned/adam/random",
        "scope",
        "main",
        "scope_cli_secret",
        Some(1),
    );

    assert_eq!(
        plan.args,
        vec![
            "-c",
            "protocol.version=2",
            "fetch",
            "--no-tags",
            "https://scope.example/git/permissioned/adam/random",
            "+refs/heads/main:refs/remotes/scope/main"
        ]
    );
    assert!(!plan.args.iter().any(|arg| arg.contains("scope_cli_secret")));
    assert_eq!(
        plan.env,
        vec![
            ("GIT_CONFIG_COUNT".to_string(), "2".to_string()),
            (
                "GIT_CONFIG_KEY_1".to_string(),
                "http.https://scope.example/git/permissioned/adam/random.extraHeader".to_string()
            ),
            (
                "GIT_CONFIG_VALUE_1".to_string(),
                "Authorization: Bearer scope_cli_secret".to_string()
            ),
        ]
    );
}

#[test]
fn parse_name_status_keeps_renames_readable() {
    assert_eq!(
        parse_name_status(b"A\tREADME.md\nR100\told.rs\tnew.rs\n"),
        vec![
            GitChangedPath {
                status: "A".to_string(),
                path: "README.md".to_string(),
            },
            GitChangedPath {
                status: "R100".to_string(),
                path: "old.rs -> new.rs".to_string(),
            },
        ]
    );
}

#[test]
fn parse_tree_paths_marks_first_push_files_added() {
    assert_eq!(
        parse_tree_paths_as_added(b".scope/repo.json\nREADME.md\n"),
        vec![
            GitChangedPath {
                status: "A".to_string(),
                path: ".scope/repo.json".to_string(),
            },
            GitChangedPath {
                status: "A".to_string(),
                path: "README.md".to_string(),
            },
        ]
    );
}

#[test]
fn parse_nul_paths_ignores_empty_entries() {
    assert_eq!(
        parse_nul_paths(b" README.md\0src/main.rs \0\0"),
        vec![" README.md".to_string(), "src/main.rs ".to_string()]
    );
}

#[test]
fn exclude_deleted_paths_removes_unstaged_deleted_tracked_files() {
    assert_eq!(
        exclude_deleted_paths(
            vec![
                "README.md".to_string(),
                "old.rs".to_string(),
                "new.rs".to_string(),
            ],
            vec!["old.rs".to_string()],
        ),
        vec!["README.md".to_string(), "new.rs".to_string()]
    );
}
