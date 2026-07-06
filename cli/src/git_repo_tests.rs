use super::*;

#[test]
fn git_push_auth_plan_keeps_bearer_token_out_of_process_args() {
    let plan = git_push_auth_plan(
        "https://scope.example/git/adam/random",
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
            "https://scope.example/git/adam/random",
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
                "http.https://scope.example/git/adam/random.extraHeader".to_string()
            ),
            (
                "GIT_CONFIG_VALUE_2".to_string(),
                "Authorization: Bearer scope_cli_secret".to_string()
            ),
            (
                "GIT_CONFIG_KEY_3".to_string(),
                "http.https://scope.example/git/adam/random.extraHeader".to_string()
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
        "https://scope.example/git/adam/random",
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
            "https://scope.example/git/adam/random",
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
                "http.https://scope.example/git/adam/random.extraHeader".to_string()
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
