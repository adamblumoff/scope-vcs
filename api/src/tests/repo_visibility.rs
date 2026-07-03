use super::*;

#[tokio::test]
async fn published_default_private_repo_serves_public_file_subset() {
    let state = test_state_with_repo();
    {
        let mut repo = test_repo(&test_owner_id());
        repo.record.default_visibility = Visibility::Private;
        repo.policy = Policy::new(Visibility::Private);
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
            changes: vec![
                FileChange {
                    visibility: Visibility::Public,
                    path: ScopePath::parse("/README.md").unwrap(),
                    old_content: None,
                    new_content: Some(source_blob("hello")),
                },
                FileChange {
                    visibility: Visibility::Private,
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
async fn published_repo_projection_preview_serves_public_file_subset() {
    let state = test_state_with_repo();
    {
        let mut repo = test_repo(&test_owner_id());
        repo.record.default_visibility = Visibility::Private;
        repo.policy = Policy::new(Visibility::Private);
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
            changes: vec![
                FileChange {
                    visibility: Visibility::Public,
                    path: ScopePath::parse("/README.md").unwrap(),
                    old_content: None,
                    new_content: Some(source_blob("hello")),
                },
                FileChange {
                    visibility: Visibility::Private,
                    path: ScopePath::parse("/secret.txt").unwrap(),
                    old_content: None,
                    new_content: Some(source_blob("secret")),
                },
            ],
        });
        repo.graph.commits.push(LogicalCommit {
            id: "rv2".to_string(),
            parent_ids: vec!["rv1".to_string()],
            author_id: repo.record.owner_user_id.clone(),
            author_visibility: AuthorVisibility::Visible,
            message: "private notes".to_string(),
            changes: vec![FileChange {
                visibility: Visibility::Private,
                path: ScopePath::parse("/notes/private.md").unwrap(),
                old_content: None,
                new_content: Some(source_blob("private notes")),
            }],
        });

        let mut catalog = lock_catalog(&state).unwrap();
        catalog.repositories.insert(TEST_REPO_ID.to_string(), repo);
    }

    cache_test_jwks(&state);
    let public_response = router(state.clone())
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/v1/repos/owner/repo/projection-preview?audience=public")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(public_response.status(), StatusCode::OK);
    let public_body = response_json(public_response).await;
    assert_eq!(public_body["audience"], "public");
    assert_eq!(public_body["source"], "live");
    assert_eq!(public_body["summary"]["visible_files"], 1);
    assert_eq!(public_body["summary"]["hidden_files"], 0);
    assert_eq!(public_body["summary"]["hidden_commits"], 0);
    assert_eq!(public_body["files"][0]["path"], "/README.md");

    let owner_response = router(state)
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/v1/repos/owner/repo/projection-preview?audience=public")
                .header(AUTHORIZATION, bearer_header())
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(owner_response.status(), StatusCode::OK);
    let owner_body = response_json(owner_response).await;
    assert_eq!(owner_body["summary"]["visible_files"], 1);
    assert_eq!(owner_body["summary"]["hidden_files"], 2);
    assert_eq!(owner_body["summary"]["hidden_commits"], 1);
}

#[tokio::test]
async fn owner_projection_preview_labels_mixed_visibility_commit() {
    let state = test_state_with_repo();
    cache_test_jwks(&state);
    {
        let mut repo = test_repo(&test_owner_id());
        repo.policy
            .add_rule(VisibilityRule::private(
                ScopePath::parse("/secret.txt").unwrap(),
            ))
            .unwrap();
        repo.graph.commits.push(LogicalCommit {
            id: "rv1".to_string(),
            parent_ids: Vec::new(),
            author_id: repo.record.owner_user_id.clone(),
            author_visibility: AuthorVisibility::Visible,
            message: "mixed visibility".to_string(),
            changes: vec![
                FileChange {
                    visibility: Visibility::Public,
                    path: ScopePath::parse("/README.md").unwrap(),
                    old_content: None,
                    new_content: Some(source_blob("hello")),
                },
                FileChange {
                    visibility: Visibility::Private,
                    path: ScopePath::parse("/secret.txt").unwrap(),
                    old_content: None,
                    new_content: Some(source_blob("secret")),
                },
            ],
        });

        let mut catalog = lock_catalog(&state).unwrap();
        catalog.repositories.insert(TEST_REPO_ID.to_string(), repo);
    }

    let response = router(state)
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/v1/repos/owner/repo/projection-preview?audience=private")
                .header(AUTHORIZATION, bearer_header())
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let body = response_json(response).await;
    assert_eq!(body["commits"][0]["visibility"], "Mixed");
}

#[tokio::test]
async fn owner_public_projection_preview_counts_visibility_transition_hidden_commits() {
    let state = test_state_with_repo();
    cache_test_jwks(&state);
    {
        let mut repo = test_repo(&test_owner_id());
        repo.record.default_visibility = Visibility::Private;
        repo.policy = Policy::new(Visibility::Private);
        repo.policy
            .add_rule(VisibilityRule::public(
                ScopePath::parse("/notes.md").unwrap(),
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
                path: ScopePath::parse("/notes.md").unwrap(),
                old_content: None,
                new_content: Some(private_blob.clone()),
            }],
        });
        repo.visibility_events.push(VisibilityEvent {
            id: "vis_1".to_string(),
            after_commit_id: Some("rv1".to_string()),
            source_commit_id: None,
            author_id: repo.record.owner_user_id.clone(),
            path: ScopePath::parse("/notes.md").unwrap(),
            old_visibility: Visibility::Private,
            new_visibility: Visibility::Public,
            current_content: Some(private_blob),
        });

        let mut catalog = lock_catalog(&state).unwrap();
        catalog.repositories.insert(TEST_REPO_ID.to_string(), repo);
    }

    let response = router(state)
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/v1/repos/owner/repo/projection-preview?audience=public")
                .header(AUTHORIZATION, bearer_header())
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let body = response_json(response).await;
    assert_eq!(body["summary"]["visible_commits"], 1);
    assert_eq!(body["summary"]["hidden_commits"], 1);
    assert_eq!(body["commits"][0]["logical_commit_id"], "vis_1");
    assert_eq!(body["commits"][0]["visibility"], "FullyPublic");
}

#[tokio::test]
async fn published_default_private_repo_without_public_files_stays_hidden() {
    let state = test_state_with_repo();
    {
        let mut repo = test_repo(&test_owner_id());
        repo.record.default_visibility = Visibility::Private;
        repo.policy = Policy::new(Visibility::Private);
        repo.graph.commits.push(LogicalCommit {
            id: "rv1".to_string(),
            parent_ids: Vec::new(),
            author_id: repo.record.owner_user_id.clone(),
            author_visibility: AuthorVisibility::Visible,
            message: "initial".to_string(),
            changes: vec![FileChange {
                visibility: Visibility::Private,
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
async fn logged_in_non_member_reads_empty_public_repo_as_public() {
    let state = test_state_with_repo();
    cache_test_jwks(&state);
    let app = router(state);
    let other_auth = bearer_header_for("user_other", "other@example.com");

    let repo_response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/v1/repos/owner/repo")
                .header(AUTHORIZATION, other_auth.as_str())
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(repo_response.status(), StatusCode::OK);
    let body = response_json(repo_response).await;
    assert_eq!(body["id"], TEST_REPO_ID);
    assert_eq!(body["access"]["actor"], "Public");
    assert_eq!(body["change_version"], 0);

    let files_response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/v1/repos/owner/repo/files")
                .header(AUTHORIZATION, other_auth.as_str())
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(files_response.status(), StatusCode::OK);
    assert!(
        response_json(files_response)
            .await
            .as_array()
            .unwrap()
            .is_empty()
    );

    let settings_response = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/v1/repos/owner/repo/settings")
                .header(AUTHORIZATION, other_auth)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(settings_response.status(), StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn deleted_public_file_no_longer_makes_private_repo_visible() {
    let state = test_state_with_repo();
    {
        let mut repo = test_repo(&test_owner_id());
        repo.record.default_visibility = Visibility::Private;
        repo.policy = Policy::new(Visibility::Private);
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
            changes: vec![FileChange {
                visibility: Visibility::Public,
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
            changes: vec![FileChange {
                visibility: Visibility::Public,
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
    let principal = principal_for_scope_user(&repo, None);

    assert_eq!(principal, Principal::public());
}

#[test]
fn scope_owner_uses_repo_principal() {
    let state = test_state_with_repo();
    let repo = find_repo(&state, TEST_REPO_OWNER, TEST_REPO_NAME).unwrap();
    let user = UserAccount {
        id: test_owner_id(),
        handle: TEST_REPO_OWNER.to_string(),
        email: TEST_OWNER_EMAIL.to_string(),
        email_verified: true,
        access: AccountAccess::Member,
    };
    let principal = principal_for_scope_user(&repo, Some(&user));

    assert_eq!(principal.id, test_owner_id());
    assert_eq!(principal.kind, PrincipalKind::User);
}

#[test]
fn non_member_scope_user_uses_public_principal() {
    let state = test_state_with_repo();
    let repo = find_repo(&state, TEST_REPO_OWNER, TEST_REPO_NAME).unwrap();
    let user = UserAccount {
        id: "scope_usr_other".to_string(),
        handle: "other".to_string(),
        email: "other@example.com".to_string(),
        email_verified: true,
        access: AccountAccess::Member,
    };
    let principal = principal_for_scope_user(&repo, Some(&user));

    assert_eq!(principal, Principal::public());
}

#[test]
fn unreadable_repo_is_hidden_from_public_requests() {
    let state = test_state_with_repo();
    let mut repo = find_repo(&state, TEST_REPO_OWNER, TEST_REPO_NAME)
        .unwrap()
        .clone();
    repo.record.publication_state = RepoPublicationState::Unpublished;

    let error = ensure_repo_read(&state, &repo, &Principal::public()).unwrap_err();

    assert_eq!(error.status, StatusCode::NOT_FOUND);
}

#[test]
fn git_projection_cache_omits_private_files_for_public_clone() {
    let owner_id = test_owner_id();
    let mut policy = Policy::new(Visibility::Public);
    policy
        .add_rule(VisibilityRule::private(
            ScopePath::parse("/secret.txt").unwrap(),
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
            changes: vec![
                FileChange {
                    visibility: Visibility::Public,
                    path: ScopePath::parse("/README.md").unwrap(),
                    old_content: None,
                    new_content: Some(source_blob("hello")),
                },
                FileChange {
                    visibility: Visibility::Private,
                    path: ScopePath::parse("/secret.txt").unwrap(),
                    old_content: None,
                    new_content: Some(source_blob("nope")),
                },
            ],
        }],
    };
    let projection = project_graph(&policy, &graph, &[], ProjectionViewKey::Public);
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

#[test]
fn git_projection_cache_preserves_executable_file_mode() {
    let owner_id = test_owner_id();
    let policy = Policy::new(Visibility::Public);
    let mut script = source_blob("#!/bin/sh\necho hi\n");
    script.git_file_mode = EXECUTABLE_GIT_FILE_MODE.to_string();
    let graph = SourceGraph {
        repo_id: TEST_REPO_ID.to_string(),
        commits: vec![LogicalCommit {
            id: "rv1".to_string(),
            parent_ids: Vec::new(),
            author_id: owner_id,
            author_visibility: AuthorVisibility::Visible,
            message: "initial".to_string(),
            changes: vec![FileChange {
                visibility: Visibility::Public,
                path: ScopePath::parse("/bin/run").unwrap(),
                old_content: None,
                new_content: Some(script),
            }],
        }],
    };
    let projection = project_graph(&policy, &graph, &[], ProjectionViewKey::Public);
    let cache_root = std::env::temp_dir().join(format!(
        "scope-vcs-git-mode-cache-test-{}-{}",
        std::process::id(),
        unix_now()
    ));
    let _ = fs::remove_dir_all(&cache_root);
    ensure_private_dir(&cache_root).unwrap();

    let repo_path =
        projection_bare_repo(&MemoryObjectStore::new(), &cache_root, &projection).unwrap();
    let tree = git_stdout_text(
        &repo_path,
        &["ls-tree", "-r", DEFAULT_GIT_BRANCH],
        "list cached projection modes",
    )
    .unwrap();

    assert!(tree.contains("100755 blob"));
    assert!(tree.contains("bin/run"));
    let _ = fs::remove_dir_all(&cache_root);
}

#[test]
fn public_git_projection_starts_at_private_to_public_transition() {
    let owner_id = test_owner_id();
    let mut policy = Policy::new(Visibility::Private);
    policy
        .add_rule(VisibilityRule::public(
            ScopePath::parse("/notes.md").unwrap(),
        ))
        .unwrap();
    let private_blob = source_blob("private draft");
    let graph = SourceGraph {
        repo_id: TEST_REPO_ID.to_string(),
        commits: vec![
            LogicalCommit {
                id: "rv1".to_string(),
                parent_ids: Vec::new(),
                author_id: owner_id.clone(),
                author_visibility: AuthorVisibility::Visible,
                message: "private draft".to_string(),
                changes: vec![FileChange {
                    visibility: Visibility::Private,
                    path: ScopePath::parse("/notes.md").unwrap(),
                    old_content: None,
                    new_content: Some(private_blob.clone()),
                }],
            },
            LogicalCommit {
                id: "rv2".to_string(),
                parent_ids: vec!["rv1".to_string()],
                author_id: owner_id,
                author_visibility: AuthorVisibility::Visible,
                message: "public release".to_string(),
                changes: vec![FileChange {
                    visibility: Visibility::Public,
                    path: ScopePath::parse("/notes.md").unwrap(),
                    old_content: Some(private_blob),
                    new_content: Some(source_blob("public release")),
                }],
            },
        ],
    };
    let projection = project_graph(&policy, &graph, &[], ProjectionViewKey::Public);
    let cache_root = std::env::temp_dir().join(format!(
        "scope-vcs-git-transition-test-{}-{}",
        std::process::id(),
        unix_now()
    ));
    let _ = fs::remove_dir_all(&cache_root);
    ensure_private_dir(&cache_root).unwrap();

    let repo_path =
        projection_bare_repo(&MemoryObjectStore::new(), &cache_root, &projection).unwrap();
    let history = git_stdout_text(
        &repo_path,
        &["log", "--all", "-p", "--", "notes.md"],
        "read projected Git history",
    )
    .unwrap();

    assert!(!history.contains("private draft"));
    let _ = fs::remove_dir_all(&cache_root);
}

#[test]
fn public_git_projection_drops_history_after_public_to_private_transition() {
    let owner_id = test_owner_id();
    let mut policy = Policy::new(Visibility::Public);
    policy
        .add_rule(VisibilityRule::private(
            ScopePath::parse("/README.md").unwrap(),
        ))
        .unwrap();
    let public_blob = source_blob("public readme");
    let graph = SourceGraph {
        repo_id: TEST_REPO_ID.to_string(),
        commits: vec![
            LogicalCommit {
                id: "rv1".to_string(),
                parent_ids: Vec::new(),
                author_id: owner_id.clone(),
                author_visibility: AuthorVisibility::Visible,
                message: "public readme".to_string(),
                changes: vec![FileChange {
                    visibility: Visibility::Public,
                    path: ScopePath::parse("/README.md").unwrap(),
                    old_content: None,
                    new_content: Some(public_blob.clone()),
                }],
            },
            LogicalCommit {
                id: "rv2".to_string(),
                parent_ids: vec!["rv1".to_string()],
                author_id: owner_id,
                author_visibility: AuthorVisibility::Visible,
                message: "private readme".to_string(),
                changes: vec![FileChange {
                    visibility: Visibility::Private,
                    path: ScopePath::parse("/README.md").unwrap(),
                    old_content: Some(public_blob),
                    new_content: Some(source_blob("private readme")),
                }],
            },
        ],
    };
    let projection = project_graph(&policy, &graph, &[], ProjectionViewKey::Public);
    let cache_root = std::env::temp_dir().join(format!(
        "scope-vcs-git-public-to-private-test-{}-{}",
        std::process::id(),
        unix_now()
    ));
    let _ = fs::remove_dir_all(&cache_root);
    ensure_private_dir(&cache_root).unwrap();

    let repo_path =
        projection_bare_repo(&MemoryObjectStore::new(), &cache_root, &projection).unwrap();
    let history = git_stdout_text(
        &repo_path,
        &["log", "--all", "--format=%B", "--", "README.md"],
        "read projected Git history",
    )
    .unwrap();
    let tree = git_stdout_text(
        &repo_path,
        &["ls-tree", "-r", "--name-only", DEFAULT_GIT_BRANCH],
        "list projected Git tree",
    )
    .unwrap();

    assert!(!history.contains("public readme"));
    assert!(!tree.contains("README.md"));
    let _ = fs::remove_dir_all(&cache_root);
}
