use super::*;
use crate::test_support::TestDir;
use std::{fs, path::Path, process::Command};

const REMOTE: &str = "https://scope.example/git/permissioned/adam/random";

#[test]
fn authenticated_git_plans_keep_secrets_in_numbered_environment_config() {
    assert_auth_plan(
        git_clone_auth_plan(
            REMOTE,
            "scope_cli_secret",
            Some(Path::new("local-dir")),
            Some(2),
        ),
        &["clone", REMOTE, "local-dir"],
        2,
        &["Authorization: Bearer scope_cli_secret"],
    );
    assert_auth_plan(
        git_fetch_auth_plan(REMOTE, "scope", "main", "scope_cli_secret", Some(1)),
        &[
            "-c",
            "protocol.version=2",
            "fetch",
            "--no-tags",
            REMOTE,
            "+refs/heads/main:refs/remotes/scope/main",
        ],
        1,
        &["Authorization: Bearer scope_cli_secret"],
    );
    assert_auth_plan(
        git_push_auth_plan(
            REMOTE,
            "1234567890123456789012345678901234567890",
            "main",
            "scope_cli_secret",
            "scope_pi_secret",
            Some(2),
        ),
        &[
            "-c",
            "push.recurseSubmodules=no",
            "push",
            REMOTE,
            "1234567890123456789012345678901234567890:refs/heads/main",
        ],
        2,
        &[
            "Authorization: Bearer scope_cli_secret",
            "X-Scope-Push-Intent: scope_pi_secret",
        ],
    );
}

fn assert_auth_plan(plan: GitCommandPlan, args: &[&str], inherited_count: usize, headers: &[&str]) {
    assert_eq!(plan.args, args);
    assert!(!plan.args.iter().any(|arg| arg.contains("secret")));
    assert_eq!(
        plan.env[0],
        (
            "GIT_CONFIG_COUNT".into(),
            (inherited_count + headers.len()).to_string()
        )
    );
    for (offset, header) in headers.iter().enumerate() {
        let index = inherited_count + offset;
        assert_eq!(
            plan.env[offset * 2 + 1],
            (
                format!("GIT_CONFIG_KEY_{index}"),
                format!("http.{REMOTE}.extraHeader"),
            )
        );
        assert_eq!(
            plan.env[offset * 2 + 2],
            (format!("GIT_CONFIG_VALUE_{index}"), (*header).to_string(),)
        );
    }
}

#[test]
fn install_scope_fetch_auth_writes_secret_free_credential_helper_for_permissioned_remote() {
    let dir = TestDir::git_repo("scope-fetch-auth", "main");
    let root = dir.path();
    let remote_url = "https://scope.example/git/permissioned/adam/random";

    install_scope_fetch_auth(root, remote_url).unwrap();
    install_scope_fetch_auth(root, remote_url).unwrap();

    let helpers = git_config(
        root,
        &["--get-all", &format!("credential.{remote_url}.helper")],
    );
    assert_eq!(
        helpers.lines().collect::<Vec<_>>(),
        vec!["", SCOPE_GIT_CREDENTIAL_HELPER]
    );
    assert_eq!(
        git_config(root, &["--get-urlmatch", "credential.helper", remote_url]),
        SCOPE_GIT_CREDENTIAL_HELPER
    );
    assert_eq!(
        git_config(
            root,
            &["--get-urlmatch", "credential.useHttpPath", remote_url]
        ),
        "true"
    );
    let config = fs::read_to_string(root.join(".git/config")).unwrap();
    assert!(
        !config.contains("scope_cli_secret"),
        "repo config must not persist Scope session tokens"
    );
}

fn git_config(root: &Path, args: &[&str]) -> String {
    let output = Command::new("git")
        .current_dir(root)
        .args(["config", "--local"])
        .args(args)
        .output()
        .unwrap();
    assert!(output.status.success());
    String::from_utf8(output.stdout)
        .unwrap()
        .trim_end()
        .to_string()
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
