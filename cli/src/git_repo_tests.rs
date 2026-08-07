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

#[test]
fn dirty_detection_includes_untracked_workflow_definitions() {
    assert!(has_dirty_paths(b"?? .scope/runs/test.yml\n"));
    assert!(has_dirty_paths(b" M README.md\n"));
    assert!(!has_dirty_paths(b""));
}

#[test]
fn request_side_paths_use_the_merge_base_after_main_diverges() {
    let dir = request_repo("request-side-diverged");
    fs::write(dir.path().join("README.md"), "base\n").unwrap();
    commit_all(&dir, "base");
    let recorded_base_oid = oid(&dir);
    dir.run_git(["branch", "request"]);

    fs::create_dir_all(dir.path().join(".scope")).unwrap();
    fs::write(dir.path().join(".scope/RULES.md"), "main only\n").unwrap();
    commit_all(&dir, "advance main");
    let current_main_oid = oid(&dir);

    dir.run_git(["checkout", "request"]);
    fs::create_dir_all(dir.path().join("src")).unwrap();
    fs::write(dir.path().join("src/lib.rs"), "request only\n").unwrap();
    commit_all(&dir, "advance request");
    let request_head_oid = oid(&dir);

    assert_eq!(
        request_side_changed_file_paths(
            &repo(&dir),
            &recorded_base_oid,
            &current_main_oid,
            &request_head_oid,
        )
        .unwrap(),
        ["src/lib.rs"]
    );
}

#[test]
fn request_side_paths_follow_main_side_renames() {
    let dir = request_repo("request-side-main-rename");
    fs::create_dir_all(dir.path().join("docs")).unwrap();
    fs::write(dir.path().join("docs/rules"), "base\n").unwrap();
    commit_all(&dir, "base");
    let recorded_base_oid = oid(&dir);
    dir.run_git(["branch", "request"]);

    fs::create_dir_all(dir.path().join(".scope")).unwrap();
    dir.run_git(["mv", "docs/rules", ".scope/rules"]);
    commit_all(&dir, "move rules into protected paths");
    let current_main_oid = oid(&dir);

    dir.run_git(["checkout", "request"]);
    fs::write(dir.path().join("docs/rules"), "request edit\n").unwrap();
    commit_all(&dir, "edit rules at the request path");
    let request_head_oid = oid(&dir);

    assert_eq!(
        request_side_changed_file_paths(
            &repo(&dir),
            &recorded_base_oid,
            &current_main_oid,
            &request_head_oid,
        )
        .unwrap(),
        [".scope/rules", "docs/rules"]
    );
}

#[test]
fn request_side_paths_keep_conflicted_protected_path_changes() {
    let dir = request_repo("request-side-protected-conflict");
    fs::create_dir_all(dir.path().join(".scope")).unwrap();
    fs::write(dir.path().join(".scope/rules"), "base\n").unwrap();
    commit_all(&dir, "base");
    let recorded_base_oid = oid(&dir);
    dir.run_git(["branch", "request"]);

    fs::write(dir.path().join(".scope/rules"), "main edit\n").unwrap();
    commit_all(&dir, "edit protected rules on main");
    let current_main_oid = oid(&dir);

    dir.run_git(["checkout", "request"]);
    fs::remove_file(dir.path().join(".scope/rules")).unwrap();
    commit_all(&dir, "delete protected rules in request");
    let request_head_oid = oid(&dir);

    assert_eq!(
        request_side_changed_file_paths(
            &repo(&dir),
            &recorded_base_oid,
            &current_main_oid,
            &request_head_oid,
        )
        .unwrap(),
        [".scope/rules"]
    );
}

#[test]
fn request_side_paths_allow_conflicted_ordinary_path_changes() {
    let dir = request_repo("request-side-ordinary-conflict");
    fs::write(dir.path().join("README.md"), "base\n").unwrap();
    commit_all(&dir, "base");
    let recorded_base_oid = oid(&dir);
    dir.run_git(["branch", "request"]);

    fs::write(dir.path().join("README.md"), "main edit\n").unwrap();
    commit_all(&dir, "edit readme on main");
    let current_main_oid = oid(&dir);

    dir.run_git(["checkout", "request"]);
    fs::write(dir.path().join("README.md"), "request edit\n").unwrap();
    commit_all(&dir, "edit readme in request");
    let request_head_oid = oid(&dir);

    assert_eq!(
        request_side_changed_file_paths(
            &repo(&dir),
            &recorded_base_oid,
            &current_main_oid,
            &request_head_oid,
        )
        .unwrap(),
        ["README.md"]
    );
}

#[test]
fn request_side_paths_support_fast_forward_history() {
    let dir = request_repo("request-side-fast-forward");
    fs::write(dir.path().join("README.md"), "base\n").unwrap();
    commit_all(&dir, "base");
    let current_main_oid = oid(&dir);
    let recorded_base_oid = current_main_oid.clone();

    fs::create_dir_all(dir.path().join("src")).unwrap();
    fs::write(dir.path().join("src/lib.rs"), "request only\n").unwrap();
    commit_all(&dir, "advance request");
    let request_head_oid = oid(&dir);

    assert_eq!(
        request_side_changed_file_paths(
            &repo(&dir),
            &recorded_base_oid,
            &current_main_oid,
            &request_head_oid,
        )
        .unwrap(),
        ["src/lib.rs"]
    );
}

#[test]
fn request_side_paths_keep_the_request_delta_after_main_is_merged() {
    let dir = request_repo("request-side-merged-main");
    fs::write(dir.path().join("README.md"), "base\n").unwrap();
    commit_all(&dir, "base");
    let recorded_base_oid = oid(&dir);
    dir.run_git(["branch", "request"]);

    fs::create_dir_all(dir.path().join(".scope")).unwrap();
    fs::write(dir.path().join(".scope/RULES.md"), "main only\n").unwrap();
    commit_all(&dir, "advance main");
    let current_main_oid = oid(&dir);

    dir.run_git(["checkout", "request"]);
    fs::create_dir_all(dir.path().join("src")).unwrap();
    fs::write(dir.path().join("src/lib.rs"), "request only\n").unwrap();
    commit_all(&dir, "advance request");
    dir.run_git(["merge", "--no-edit", "main"]);
    let request_head_oid = oid(&dir);

    assert_eq!(
        request_side_changed_file_paths(
            &repo(&dir),
            &recorded_base_oid,
            &current_main_oid,
            &request_head_oid,
        )
        .unwrap(),
        ["src/lib.rs"]
    );
}

#[test]
fn request_side_paths_name_missing_local_commits() {
    let dir = request_repo("request-side-missing");
    fs::write(dir.path().join("README.md"), "base\n").unwrap();
    commit_all(&dir, "base");
    let existing_oid = oid(&dir);

    let missing_base =
        request_side_changed_file_paths(&repo(&dir), "missing-base", &existing_oid, &existing_oid)
            .unwrap_err();
    assert!(
        missing_base
            .to_string()
            .contains("recorded request base commit is missing")
    );

    let missing_main =
        request_side_changed_file_paths(&repo(&dir), &existing_oid, "missing-main", &existing_oid)
            .unwrap_err();
    assert!(
        missing_main
            .to_string()
            .contains("current main commit is missing")
    );

    let missing_head =
        request_side_changed_file_paths(&repo(&dir), &existing_oid, &existing_oid, "missing-head")
            .unwrap_err();
    assert!(
        missing_head
            .to_string()
            .contains("request head commit is missing")
    );
}

#[test]
fn request_side_paths_reject_unrelated_histories() {
    let dir = request_repo("request-side-unrelated");
    fs::write(dir.path().join("README.md"), "main\n").unwrap();
    commit_all(&dir, "main root");
    let current_main_oid = oid(&dir);

    dir.run_git(["checkout", "--orphan", "request"]);
    dir.run_git(["rm", "-rf", "."]);
    fs::write(dir.path().join("request.txt"), "unrelated\n").unwrap();
    commit_all(&dir, "request root");
    let request_head_oid = oid(&dir);

    let error = request_side_changed_file_paths(
        &repo(&dir),
        &current_main_oid,
        &current_main_oid,
        &request_head_oid,
    )
    .unwrap_err();
    let message = error.to_string();
    assert!(message.contains("unrelated Git histories"));
    assert!(message.contains("scope request start <name>"));
    assert!(message.contains("replay the GitHub branch changes"));
}

#[test]
fn request_side_paths_reject_a_merge_base_outside_the_recorded_request_history() {
    let dir = request_repo("request-side-invalid-recorded-base");
    fs::write(dir.path().join("README.md"), "root\n").unwrap();
    commit_all(&dir, "root");
    dir.run_git(["branch", "invalid-base"]);

    fs::write(dir.path().join("README.md"), "main\n").unwrap();
    commit_all(&dir, "main");
    let current_main_oid = oid(&dir);
    fs::write(dir.path().join("request.txt"), "request\n").unwrap();
    commit_all(&dir, "request");
    let request_head_oid = oid(&dir);

    dir.run_git(["checkout", "invalid-base"]);
    fs::write(dir.path().join("invalid.txt"), "invalid base\n").unwrap();
    commit_all(&dir, "invalid base");
    let recorded_base_oid = oid(&dir);

    let error = request_side_changed_file_paths(
        &repo(&dir),
        &recorded_base_oid,
        &current_main_oid,
        &request_head_oid,
    )
    .unwrap_err();
    assert!(
        error
            .to_string()
            .contains("merge base does not descend from the recorded request base")
    );
}

#[test]
fn request_side_paths_reject_criss_cross_histories_with_multiple_merge_bases() {
    let dir = request_repo("request-side-multiple-merge-bases");
    fs::write(dir.path().join("README.md"), "root\n").unwrap();
    commit_all(&dir, "root");
    let recorded_base_oid = oid(&dir);
    dir.run_git(["branch", "left"]);
    dir.run_git(["branch", "right"]);

    dir.run_git(["checkout", "left"]);
    fs::write(dir.path().join("left.txt"), "left\n").unwrap();
    commit_all(&dir, "left");
    dir.run_git(["branch", "left-tip"]);

    dir.run_git(["checkout", "right"]);
    fs::write(dir.path().join("right.txt"), "right\n").unwrap();
    commit_all(&dir, "right");
    dir.run_git(["branch", "right-tip"]);

    dir.run_git(["checkout", "left"]);
    dir.run_git(["merge", "--no-edit", "right-tip"]);
    let current_main_oid = oid(&dir);

    dir.run_git(["checkout", "right"]);
    dir.run_git(["merge", "--no-edit", "left-tip"]);
    let request_head_oid = oid(&dir);
    let merge_bases = String::from_utf8(
        dir.run_git(["merge-base", "--all", &current_main_oid, &request_head_oid])
            .stdout,
    )
    .unwrap();
    assert_eq!(merge_bases.lines().count(), 2);

    let error = request_side_changed_file_paths(
        &repo(&dir),
        &recorded_base_oid,
        &current_main_oid,
        &request_head_oid,
    )
    .unwrap_err();
    assert!(error.to_string().contains("multiple Git merge bases"));
}

fn request_repo(label: &str) -> TestDir {
    let dir = TestDir::git_repo(label, "main");
    dir.run_git(["config", "user.email", "scope@example.test"]);
    dir.run_git(["config", "user.name", "Scope Test"]);
    dir
}

fn commit_all(dir: &TestDir, message: &str) {
    dir.run_git(["add", "-A"]);
    let output = Command::new("git")
        .current_dir(dir.path())
        .args(["commit", "-m", message])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "git commit failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

fn oid(dir: &TestDir) -> String {
    String::from_utf8(dir.run_git(["rev-parse", "HEAD"]).stdout)
        .unwrap()
        .trim()
        .to_string()
}

fn repo(dir: &TestDir) -> GitRepo {
    GitRepo {
        root: dir.path().to_path_buf(),
    }
}
