use super::*;

#[derive(Clone, Copy)]
enum PrivacyHistory {
    Mixed,
    Revealed,
    Deleted,
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn private_history_shapes_cannot_enter_public_request_refs() {
    for (history, label) in [
        (
            PrivacyHistory::Mixed,
            "request-ref-maintainer-private-history",
        ),
        (
            PrivacyHistory::Revealed,
            "request-ref-public-base-private-side-history",
        ),
        (
            PrivacyHistory::Deleted,
            "request-ref-public-base-deleted-private-side-history",
        ),
    ] {
        eprintln!("privacy history case: {label}");
        assert_private_history_push_rejected(history, label).await;
    }
}

async fn assert_private_history_push_rejected(history: PrivacyHistory, source_label: &str) {
    let state = test_state_with_request().await;
    state
        .metadata
        .replace_repository_for_tests(privacy_repo(&state, history))
        .await
        .unwrap();
    insert_member_user(&state).await;
    let (origin, _server) = spawn_test_server(&state).await;

    let source = checkout_dir(source_label);
    let permissioned_remote = format!("{origin}/git/permissioned/{TEST_REPO_ID}");
    match history {
        PrivacyHistory::Mixed => clone_with_bearer(
            &permissioned_remote,
            &source,
            &bearer_header_for(MEMBER_SUBJECT, MEMBER_EMAIL),
            "clone private repo for public request",
        ),
        PrivacyHistory::Revealed | PrivacyHistory::Deleted => {
            let private_source = checkout_dir(&format!("{source_label}-private-source"));
            let public_remote = format!("{origin}/git/public/{TEST_REPO_ID}");
            run_git(
                None,
                &["clone", &public_remote, source.to_str().unwrap()],
                "clone public repo for private history request",
            )
            .unwrap();
            clone_with_bearer(
                &permissioned_remote,
                &private_source,
                &bearer_header_for(MEMBER_SUBJECT, MEMBER_EMAIL),
                "clone private history source",
            );
            run_git(
                Some(&source),
                &["remote", "add", "private", private_source.to_str().unwrap()],
                "add private history remote",
            )
            .unwrap();
            run_git(
                Some(&source),
                &["fetch", "private", "main"],
                "fetch private history",
            )
            .unwrap();
            run_git(
                Some(&source),
                &[
                    "-c",
                    "user.name=Scope Test",
                    "-c",
                    "user.email=scope-test@example.test",
                    "merge",
                    "--allow-unrelated-histories",
                    "-s",
                    "ours",
                    "--no-edit",
                    "private/main",
                ],
                "merge private history into public request branch",
            )
            .unwrap();
        }
    }
    fs::write(source.join("request.txt"), "request edit\n").unwrap();
    run_git(Some(&source), &["add", "-A"], "add request edit").unwrap();
    commit_all(&source, "request edit carrying private history");
    configure_bearer_header(
        &source,
        &permissioned_remote,
        &bearer_header_for(MEMBER_SUBJECT, MEMBER_EMAIL),
    );

    let output = run_git_output(
        Some(&source),
        &["push", &permissioned_remote, &format!("HEAD:{REQUEST_REF}")],
        "push private history to public request",
    )
    .unwrap();

    assert!(
        !output.status.success(),
        "{source_label}: private-history side branch push unexpectedly succeeded: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_request_branch_unchanged(&state).await;
}

fn privacy_repo(state: &AppState, history: PrivacyHistory) -> StoredRepository {
    let mut repo = test_repo(&test_owner_id());
    repo.graph.commits = match history {
        PrivacyHistory::Mixed => vec![history_commit(
            "rv1",
            None,
            vec![
                history_change(state, Visibility::Public, "/README.md", None, Some("hello")),
                history_change(
                    state,
                    Visibility::Private,
                    "/SECRET.md",
                    None,
                    Some("private\n"),
                ),
            ],
        )],
        PrivacyHistory::Revealed => {
            repo.policy = Policy::new(Visibility::Private);
            repo.policy
                .add_rule(VisibilityRule::public(
                    ScopePath::parse("/README.md").unwrap(),
                ))
                .unwrap();
            vec![
                history_commit(
                    "rv1",
                    None,
                    vec![history_change(
                        state,
                        Visibility::Private,
                        "/README.md",
                        None,
                        Some("private draft"),
                    )],
                ),
                history_commit(
                    "rv2",
                    Some("rv1"),
                    vec![history_change(
                        state,
                        Visibility::Public,
                        "/README.md",
                        Some("private draft"),
                        Some("public release"),
                    )],
                ),
            ]
        }
        PrivacyHistory::Deleted => vec![
            history_commit(
                "rv1",
                None,
                vec![
                    history_change(state, Visibility::Public, "/README.md", None, Some("hello")),
                    history_change(
                        state,
                        Visibility::Private,
                        "/OLD_SECRET.md",
                        None,
                        Some("deleted private\n"),
                    ),
                ],
            ),
            history_commit(
                "rv2",
                Some("rv1"),
                vec![history_change(
                    state,
                    Visibility::Private,
                    "/OLD_SECRET.md",
                    Some("deleted private\n"),
                    None,
                )],
            ),
        ],
    };
    populate_test_live_files(&mut repo);
    repo
}

fn history_commit(id: &str, parent: Option<&str>, changes: Vec<FileChange>) -> LogicalCommit {
    LogicalCommit {
        id: id.into(),
        parent_ids: parent.into_iter().map(str::to_string).collect(),
        author_id: test_owner_id(),
        author_visibility: AuthorVisibility::Visible,
        message: id.into(),
        changes,
    }
}

fn history_change(
    state: &AppState,
    visibility: Visibility,
    path: &str,
    old_content: Option<&str>,
    new_content: Option<&str>,
) -> FileChange {
    FileChange {
        visibility,
        path: ScopePath::parse(path).unwrap(),
        old_content: old_content.map(|content| source_blob(state, content)),
        new_content: new_content.map(|content| source_blob(state, content)),
    }
}
