use super::*;

#[tokio::test]
async fn published_default_private_repo_serves_public_file_subset() {
    let state = test_state_with_repo();
    {
        let mut repo = test_repo(&test_owner_id());
        repo.record.default_visibility = Visibility::Private;
        repo.policy = Policy::new(Visibility::Private, &repo.record.owner_user_id);
        repo.policy
            .add_rule(VisibilityRule::public(
                ScopePath::parse("/README.md").unwrap(),
            ))
            .unwrap();
        repo.graph.commits.push(LogicalCommit {
            id: "rv1".to_string(),
            parent_ids: Vec::new(),
            author_id: repo.record.owner_user_id.clone(),
            author_visibility: AuthorVisibility::Visible,
            message: "initial".to_string(),
            mixed_policy: MixedCommitPolicy::SyntheticPublicCommit,
            changes: vec![
                FileChange {
                    path: ScopePath::parse("/README.md").unwrap(),
                    old_content: None,
                    new_content: Some(source_blob("hello")),
                },
                FileChange {
                    path: ScopePath::parse("/secret.txt").unwrap(),
                    old_content: None,
                    new_content: Some(source_blob("secret")),
                },
            ],
        });

        let mut catalog = lock_catalog(&state).unwrap();
        catalog.repositories.insert(TEST_REPO_ID.to_string(), repo);
    }

    let app = router(state);
    let response = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/v1/repos/owner/repo/files")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let body = response_json(response).await;
    assert_eq!(body.as_array().unwrap().len(), 1);
    assert_eq!(body[0]["path"], "/README.md");
}

#[tokio::test]
async fn published_default_private_repo_without_public_files_stays_hidden() {
    let state = test_state_with_repo();
    {
        let mut repo = test_repo(&test_owner_id());
        repo.record.default_visibility = Visibility::Private;
        repo.policy = Policy::new(Visibility::Private, &repo.record.owner_user_id);
        repo.graph.commits.push(LogicalCommit {
            id: "rv1".to_string(),
            parent_ids: Vec::new(),
            author_id: repo.record.owner_user_id.clone(),
            author_visibility: AuthorVisibility::Visible,
            message: "initial".to_string(),
            mixed_policy: MixedCommitPolicy::SyntheticPublicCommit,
            changes: vec![FileChange {
                path: ScopePath::parse("/secret.txt").unwrap(),
                old_content: None,
                new_content: Some(source_blob("secret")),
            }],
        });

        let mut catalog = lock_catalog(&state).unwrap();
        catalog.repositories.insert(TEST_REPO_ID.to_string(), repo);
    }

    let app = router(state);
    let response = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/v1/repos/owner/repo/files")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn deleted_public_file_no_longer_makes_private_repo_visible() {
    let state = test_state_with_repo();
    {
        let mut repo = test_repo(&test_owner_id());
        repo.record.default_visibility = Visibility::Private;
        repo.policy = Policy::new(Visibility::Private, &repo.record.owner_user_id);
        repo.policy
            .add_rule(VisibilityRule::public(
                ScopePath::parse("/README.md").unwrap(),
            ))
            .unwrap();
        let readme_blob = source_blob("hello");
        repo.graph.commits.push(LogicalCommit {
            id: "rv1".to_string(),
            parent_ids: Vec::new(),
            author_id: repo.record.owner_user_id.clone(),
            author_visibility: AuthorVisibility::Visible,
            message: "initial".to_string(),
            mixed_policy: MixedCommitPolicy::SyntheticPublicCommit,
            changes: vec![FileChange {
                path: ScopePath::parse("/README.md").unwrap(),
                old_content: None,
                new_content: Some(readme_blob.clone()),
            }],
        });
        repo.graph.commits.push(LogicalCommit {
            id: "rv2".to_string(),
            parent_ids: vec!["rv1".to_string()],
            author_id: repo.record.owner_user_id.clone(),
            author_visibility: AuthorVisibility::Visible,
            message: "delete public file".to_string(),
            mixed_policy: MixedCommitPolicy::SyntheticPublicCommit,
            changes: vec![FileChange {
                path: ScopePath::parse("/README.md").unwrap(),
                old_content: Some(readme_blob),
                new_content: None,
            }],
        });

        let mut catalog = lock_catalog(&state).unwrap();
        catalog.repositories.insert(TEST_REPO_ID.to_string(), repo);
    }

    let app = router(state);
    let response = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/v1/repos/owner/repo/files")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}

#[test]
fn anonymous_request_uses_public_principal() {
    let state = test_state_with_repo();
    let repo = find_repo(&state, TEST_REPO_OWNER, TEST_REPO_NAME).unwrap();
    let principal = principal_for_repo(&state, &repo, None).unwrap();

    assert_eq!(principal, Principal::public());
}

#[test]
fn verified_member_email_uses_repo_principal() {
    let state = test_state_with_repo();
    let repo = find_repo(&state, TEST_REPO_OWNER, TEST_REPO_NAME).unwrap();
    let identity = owner_identity(true);
    let principal = principal_for_repo(&state, &repo, Some(&identity)).unwrap();

    assert_eq!(principal.id, test_owner_id());
    assert_eq!(principal.kind, PrincipalKind::User);
}

#[test]
fn unverified_email_still_uses_pairwise_user_principal() {
    let state = test_state_with_repo();
    let repo = find_repo(&state, TEST_REPO_OWNER, TEST_REPO_NAME).unwrap();
    let identity = owner_identity(false);
    let principal = principal_for_repo(&state, &repo, Some(&identity)).unwrap();

    assert_eq!(principal.id, test_owner_id());
    assert_eq!(principal.kind, PrincipalKind::User);
}

#[test]
fn unreadable_repo_is_hidden_from_public_requests() {
    let state = test_state_with_repo();
    let mut repo = find_repo(&state, TEST_REPO_OWNER, TEST_REPO_NAME)
        .unwrap()
        .clone();
    repo.record.publication_state = RepoPublicationState::PendingPublish;

    let error = ensure_repo_read(&state, &repo, &Principal::public()).unwrap_err();

    assert_eq!(error.status, StatusCode::NOT_FOUND);
}

#[test]
fn git_projection_cache_omits_private_files_for_public_clone() {
    let owner_id = test_owner_id();
    let mut policy = Policy::new(Visibility::Public, owner_id.clone());
    policy
        .add_rule(VisibilityRule::private(
            ScopePath::parse("/secret.txt").unwrap(),
            [owner_id.clone()],
        ))
        .unwrap();
    let graph = SourceGraph {
        repo_id: TEST_REPO_ID.to_string(),
        commits: vec![LogicalCommit {
            id: "rv1".to_string(),
            parent_ids: Vec::new(),
            author_id: owner_id,
            author_visibility: AuthorVisibility::Visible,
            message: "initial".to_string(),
            mixed_policy: MixedCommitPolicy::SyntheticPublicCommit,
            changes: vec![
                FileChange {
                    path: ScopePath::parse("/README.md").unwrap(),
                    old_content: None,
                    new_content: Some(source_blob("hello")),
                },
                FileChange {
                    path: ScopePath::parse("/secret.txt").unwrap(),
                    old_content: None,
                    new_content: Some(source_blob("nope")),
                },
            ],
        }],
    };
    let projection = project_graph(&policy, &graph, &Principal::public());
    let cache_root = std::env::temp_dir().join(format!(
        "scope-vcs-git-cache-test-{}-{}",
        std::process::id(),
        unix_now()
    ));
    let _ = fs::remove_dir_all(&cache_root);
    ensure_private_dir(&cache_root).unwrap();

    let repo_path =
        projection_bare_repo(&MemoryObjectStore::new(), &cache_root, &projection).unwrap();
    let tree = git_stdout_text(
        &repo_path,
        &["ls-tree", "-r", "--name-only", DEFAULT_GIT_BRANCH],
        "list cached projection",
    )
    .unwrap();

    assert!(tree.contains("README.md"));
    assert!(!tree.contains("secret.txt"));
    let _ = fs::remove_dir_all(&cache_root);
}
