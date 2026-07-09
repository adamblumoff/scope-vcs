use super::*;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn maintainer_cannot_push_private_history_to_public_request_ref() {
    let state = test_state_with_request_repo(repo_with_public_readme_and_private_secret());
    insert_member_user(&state);
    let state_for_server = state.clone();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        axum::serve(listener, router(state_for_server))
            .await
            .unwrap();
    });

    let source = temp_checkout_dir("request-ref-maintainer-private-history");
    let permissioned_remote = format!("http://{addr}/git/permissioned/{TEST_REPO_ID}");
    clone_with_bearer(
        &permissioned_remote,
        &source,
        &bearer_header_for(MEMBER_SUBJECT, MEMBER_EMAIL),
        "clone private repo for public request",
    );
    assert!(source.join("SECRET.md").exists());
    fs::write(source.join("maintainer.txt"), "maintainer request edit\n").unwrap();
    run_git(Some(&source), &["add", "-A"], "add maintainer request edit").unwrap();
    commit_all(&source, "maintainer request edit from private checkout");
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
        "private-history push unexpectedly succeeded: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_request_branch_unchanged(&state);

    server.abort();
    let _ = fs::remove_dir_all(source);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn maintainer_cannot_push_old_private_history_for_currently_public_path() {
    assert_private_side_history_push_rejected(
        repo_with_private_to_public_readme_history(),
        "request-ref-public-base-private-side-history",
        "push currently-public path private history to public request",
    )
    .await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn maintainer_cannot_push_deleted_private_path_history() {
    assert_private_side_history_push_rejected(
        repo_with_deleted_private_secret_history(),
        "request-ref-public-base-deleted-private-side-history",
        "push deleted private path history to public request",
    )
    .await;
}

async fn assert_private_side_history_push_rejected(
    repo: StoredRepository,
    source_label: &str,
    push_action: &str,
) {
    let state = test_state_with_request_repo(repo);
    insert_member_user(&state);
    let state_for_server = state.clone();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        axum::serve(listener, router(state_for_server))
            .await
            .unwrap();
    });

    let source = temp_checkout_dir(source_label);
    let private_source = temp_checkout_dir(&format!("{source_label}-private-source"));
    let public_remote = format!("http://{addr}/git/public/{TEST_REPO_ID}");
    let permissioned_remote = format!("http://{addr}/git/permissioned/{TEST_REPO_ID}");
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
    fs::write(source.join("request.txt"), "request edit\n").unwrap();
    run_git(Some(&source), &["add", "-A"], "add public request edit").unwrap();
    commit_all(&source, "public request edit after private side history");
    configure_bearer_header(
        &source,
        &permissioned_remote,
        &bearer_header_for(MEMBER_SUBJECT, MEMBER_EMAIL),
    );

    let output = run_git_output(
        Some(&source),
        &["push", &permissioned_remote, &format!("HEAD:{REQUEST_REF}")],
        push_action,
    )
    .unwrap();

    assert!(
        !output.status.success(),
        "private-history side branch push unexpectedly succeeded: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_request_branch_unchanged(&state);

    server.abort();
    let _ = fs::remove_dir_all(source);
    let _ = fs::remove_dir_all(private_source);
}

fn test_state_with_request_repo(repo: StoredRepository) -> AppState {
    let state = test_state_with_request();
    state
        .metadata
        .update(|catalog| {
            catalog.repositories.insert(TEST_REPO_ID.to_string(), repo);
            Ok(())
        })
        .unwrap();
    state
}

fn assert_request_branch_unchanged(state: &AppState) {
    state
        .metadata
        .read(|catalog| {
            let request = catalog.requests.get(REQUEST_ID).unwrap();
            assert_eq!(request.state, RequestState::Working);
            assert_eq!(request.head_oid, "base_main");
            assert!(request.git_snapshot.is_none());
            assert!(catalog.request_events.is_empty());
            Ok(())
        })
        .unwrap();
}

fn repo_with_public_readme_and_private_secret() -> StoredRepository {
    let mut repo = test_repo(&test_owner_id());
    repo.graph.commits.push(LogicalCommit {
        id: "rv1".to_string(),
        parent_ids: Vec::new(),
        author_id: repo.record.owner_user_id.clone(),
        author_visibility: AuthorVisibility::Visible,
        message: "initial".to_string(),
        changes: vec![
            FileChange {
                visibility: Visibility::Public,
                path: ScopePath::parse("/README.md").unwrap(),
                old_content: None,
                new_content: Some(source_blob("hello")),
            },
            FileChange {
                visibility: Visibility::Private,
                path: ScopePath::parse("/SECRET.md").unwrap(),
                old_content: None,
                new_content: Some(source_blob("private\n")),
            },
        ],
    });
    repo
}

fn repo_with_private_to_public_readme_history() -> StoredRepository {
    let mut repo = test_repo(&test_owner_id());
    repo.policy = Policy::new(Visibility::Private);
    repo.policy
        .add_rule(VisibilityRule::public(
            ScopePath::parse("/README.md").unwrap(),
        ))
        .unwrap();
    let private_blob = source_blob("private draft");
    repo.graph.commits.push(LogicalCommit {
        id: "rv1".to_string(),
        parent_ids: Vec::new(),
        author_id: repo.record.owner_user_id.clone(),
        author_visibility: AuthorVisibility::Visible,
        message: "private draft".to_string(),
        changes: vec![FileChange {
            visibility: Visibility::Private,
            path: ScopePath::parse("/README.md").unwrap(),
            old_content: None,
            new_content: Some(private_blob.clone()),
        }],
    });
    repo.graph.commits.push(LogicalCommit {
        id: "rv2".to_string(),
        parent_ids: vec!["rv1".to_string()],
        author_id: repo.record.owner_user_id.clone(),
        author_visibility: AuthorVisibility::Visible,
        message: "public release".to_string(),
        changes: vec![FileChange {
            visibility: Visibility::Public,
            path: ScopePath::parse("/README.md").unwrap(),
            old_content: Some(private_blob),
            new_content: Some(source_blob("public release")),
        }],
    });
    repo
}

fn repo_with_deleted_private_secret_history() -> StoredRepository {
    let mut repo = test_repo(&test_owner_id());
    let private_blob = source_blob("deleted private\n");
    repo.graph.commits.push(LogicalCommit {
        id: "rv1".to_string(),
        parent_ids: Vec::new(),
        author_id: repo.record.owner_user_id.clone(),
        author_visibility: AuthorVisibility::Visible,
        message: "initial with private secret".to_string(),
        changes: vec![
            FileChange {
                visibility: Visibility::Public,
                path: ScopePath::parse("/README.md").unwrap(),
                old_content: None,
                new_content: Some(source_blob("hello")),
            },
            FileChange {
                visibility: Visibility::Private,
                path: ScopePath::parse("/OLD_SECRET.md").unwrap(),
                old_content: None,
                new_content: Some(private_blob.clone()),
            },
        ],
    });
    repo.graph.commits.push(LogicalCommit {
        id: "rv2".to_string(),
        parent_ids: vec!["rv1".to_string()],
        author_id: repo.record.owner_user_id.clone(),
        author_visibility: AuthorVisibility::Visible,
        message: "delete private secret".to_string(),
        changes: vec![FileChange {
            visibility: Visibility::Private,
            path: ScopePath::parse("/OLD_SECRET.md").unwrap(),
            old_content: Some(private_blob),
            new_content: None,
        }],
    });
    repo
}
