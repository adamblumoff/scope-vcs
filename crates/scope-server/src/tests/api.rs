use super::*;

#[test]
fn test_state_starts_without_repositories() {
    let state = AppState::test_state();
    let error = find_repo(&state, TEST_REPO_OWNER, TEST_REPO_NAME).unwrap_err();

    assert_eq!(error.status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn create_repo_route_creates_user_and_lists_repo() {
    let state = test_state_with_jwks();
    let app = router(state.clone());
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/repos")
                .header(AUTHORIZATION, bearer_header())
                .header(CONTENT_TYPE, "application/json")
                .body(Body::from(r#"{"name":"Scope_App"}"#))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let body = response_json(response).await;
    assert_eq!(body["repo"]["id"], "owner/scope_app");
    assert_eq!(body["repo"]["owner_handle"], "owner");
    assert_eq!(body["repo"]["lifecycle_state"], "PendingFirstPush");
    assert_eq!(body["repo"]["default_visibility"], "Private");
    assert_eq!(body["repo"]["role"], "Owner");
    assert_eq!(body["repo"]["staged_update_pending"], false);
    assert_eq!(body["setup"]["git_remote_path"], "/git/owner/scope_app");
    assert_eq!(body["setup"]["remote_name"], "scope");
    assert_eq!(body["setup"]["push_branch"], DEFAULT_GIT_BRANCH);
    assert_eq!(body["setup"]["push_enabled"], true);
    let secret = body["setup"]["token"]["secret"].as_str().unwrap();
    assert!(secret.starts_with("scope_fp_"));
    assert_eq!(body["setup"]["token"]["status"], "Active");
    let push_secret = body["setup"]["push_token"]["secret"].as_str().unwrap();
    assert!(push_secret.starts_with("scope_git_"));

    let response = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/v1/repos")
                .header(AUTHORIZATION, bearer_header())
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let body = response_json(response).await;
    assert_eq!(body.as_array().unwrap().len(), 1);
    assert_eq!(body[0]["id"], "owner/scope_app");

    let catalog = lock_catalog(&state).unwrap();
    assert_eq!(catalog.users.len(), 1);
    assert_eq!(catalog.repositories.len(), 1);
    let repo = catalog.repositories.get("owner/scope_app").unwrap();
    let token = repo.first_push_token.as_ref().unwrap();
    assert_ne!(token.token_hash, secret);
    assert!(token.token_hash.starts_with("sha256:"));
    assert_eq!(token.owner_user_id, test_owner_id());
    assert_eq!(
        token.expires_at_unix - token.created_at_unix,
        FIRST_PUSH_TOKEN_TTL_SECS
    );
    let push_token = repo.git_push_token.as_ref().unwrap();
    assert_ne!(push_token.token_hash, push_secret);
    assert!(push_token.token_hash.starts_with("sha256:"));
    assert_eq!(push_token.owner_user_id, test_owner_id());
}

#[tokio::test]
async fn list_repos_marks_published_repo_with_staged_update() {
    let state = test_state_with_repo();
    cache_test_jwks(&state);
    {
        let mut repo = repo_with_readme();
        stage_receive_pack_update(
            &mut repo,
            receive_pack_update(vec![("/README.md", Some("staged"))]),
        )
        .unwrap();

        let mut catalog = lock_catalog(&state).unwrap();
        catalog.repositories.insert(TEST_REPO_ID.to_string(), repo);
    }

    let app = router(state);
    let summary_response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/v1/repos/owner/repo")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(summary_response.status(), StatusCode::OK);
    let summary_body = response_json(summary_response).await;
    assert_eq!(summary_body["id"], TEST_REPO_ID);
    assert_eq!(summary_body["role"], serde_json::Value::Null);

    let response = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/v1/repos")
                .header(AUTHORIZATION, bearer_header())
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let body = response_json(response).await;
    assert_eq!(body[0]["id"], TEST_REPO_ID);
    assert_eq!(body[0]["lifecycle_state"], "Published");
    assert_eq!(body[0]["staged_update_pending"], true);
}

#[tokio::test]
async fn get_repo_route_returns_owner_summary() {
    let state = test_state_with_repo();
    cache_test_jwks(&state);
    let response = router(state)
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/v1/repos/owner/repo")
                .header(AUTHORIZATION, bearer_header())
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let body = response_json(response).await;
    assert_eq!(body["id"], TEST_REPO_ID);
    assert_eq!(body["owner_handle"], TEST_REPO_OWNER);
    assert_eq!(body["name"], TEST_REPO_NAME);
    assert_eq!(body["role"], "Owner");
}

#[tokio::test]
async fn delete_repo_route_requires_owner_and_removes_storage() {
    let state = test_state_with_repo();
    cache_test_jwks(&state);
    let owner_repo = owner_git_repo_path(&state, TEST_REPO_OWNER, TEST_REPO_NAME);
    let staged_repo = staged_git_repo_path(&state, TEST_REPO_OWNER, TEST_REPO_NAME);
    let rx_repo = git_repo_storage_root(&state).join("git-rx").join(format!(
        "{}-test.git",
        receive_pack_staging_repo_prefix(TEST_REPO_OWNER, TEST_REPO_NAME)
    ));
    fs::create_dir_all(&owner_repo).unwrap();
    fs::create_dir_all(&staged_repo).unwrap();
    fs::create_dir_all(&rx_repo).unwrap();

    let app = router(state.clone());
    let non_owner = app
        .clone()
        .oneshot(
            Request::builder()
                .method("DELETE")
                .uri("/v1/repos/owner/repo")
                .header(
                    AUTHORIZATION,
                    bearer_header_for("pairwise-stranger", "stranger@example.com"),
                )
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(non_owner.status(), StatusCode::NOT_FOUND);

    let response = app
        .oneshot(
            Request::builder()
                .method("DELETE")
                .uri("/v1/repos/owner/repo")
                .header(AUTHORIZATION, bearer_header())
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let body = response_json(response).await;
    assert_eq!(body["id"], TEST_REPO_ID);
    assert_eq!(body["deleted"], true);
    assert!(find_repo(&state, TEST_REPO_OWNER, TEST_REPO_NAME).is_err());
    assert!(!owner_repo.exists());
    assert!(!staged_repo.exists());
    assert!(!rx_repo.exists());
}

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
                    new_content: Some("hello".to_string()),
                },
                FileChange {
                    path: ScopePath::parse("/secret.txt").unwrap(),
                    old_content: None,
                    new_content: Some("secret".to_string()),
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
                new_content: Some("secret".to_string()),
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
async fn setup_route_is_owner_only_and_returns_active_first_push_secret() {
    let state = test_state_with_repo();
    cache_test_jwks(&state);
    let secret = {
        let mut catalog = lock_catalog(&state).unwrap();
        let repo = catalog.repositories.get_mut(TEST_REPO_ID).unwrap();
        let (secret, token) = generate_first_push_token(&test_owner_id()).unwrap();
        repo.record.publication_state = RepoPublicationState::PendingFirstPush;
        repo.first_push_token = Some(token);
        secret
    };
    let app = router(state);

    let public_response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/v1/repos/owner/repo/setup")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(public_response.status(), StatusCode::UNAUTHORIZED);

    let non_owner_response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/v1/repos/owner/repo/setup")
                .header(
                    AUTHORIZATION,
                    bearer_header_for("pairwise-stranger", "stranger@example.com"),
                )
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(non_owner_response.status(), StatusCode::NOT_FOUND);

    let non_owner_regenerate_response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/repos/owner/repo/setup-token")
                .header(
                    AUTHORIZATION,
                    bearer_header_for("pairwise-stranger", "stranger@example.com"),
                )
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(
        non_owner_regenerate_response.status(),
        StatusCode::NOT_FOUND
    );

    let owner_response = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/v1/repos/owner/repo/setup")
                .header(AUTHORIZATION, bearer_header())
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(owner_response.status(), StatusCode::OK);
    let body = response_json(owner_response).await;
    assert_eq!(body["repo"]["id"], TEST_REPO_ID);
    assert_eq!(body["token"]["status"], "Active");
    assert_eq!(body["token"]["secret"], secret);
}

#[tokio::test]
async fn setup_token_regeneration_rotates_first_push_token_only() {
    let state = test_state_with_repo();
    cache_test_jwks(&state);
    let (old_hash, old_push_hash) = {
        let mut catalog = lock_catalog(&state).unwrap();
        let repo = catalog.repositories.get_mut(TEST_REPO_ID).unwrap();
        let (_, token) = generate_first_push_token(&test_owner_id()).unwrap();
        let (_, push_token) = generate_git_push_token(&test_owner_id()).unwrap();
        let old_hash = token.token_hash.clone();
        let old_push_hash = push_token.token_hash.clone();
        repo.record.publication_state = RepoPublicationState::PendingFirstPush;
        repo.first_push_token = Some(token);
        repo.git_push_token = Some(push_token);
        (old_hash, old_push_hash)
    };

    let response = router(state.clone())
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/repos/owner/repo/setup-token")
                .header(AUTHORIZATION, bearer_header())
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let body = response_json(response).await;
    let secret = body["token"]["secret"].as_str().unwrap();
    assert!(secret.starts_with("scope_fp_"));
    assert!(body["push_token"]["secret"].is_null());
    let catalog = lock_catalog(&state).unwrap();
    let repo = catalog.repositories.get(TEST_REPO_ID).unwrap();
    let new_hash = &repo.first_push_token.as_ref().unwrap().token_hash;
    let new_push_hash = &repo.git_push_token.as_ref().unwrap().token_hash;
    assert_ne!(new_hash, &old_hash);
    assert_ne!(new_hash, secret);
    assert_eq!(new_push_hash, &old_push_hash);
}

#[test]
fn first_push_token_response_uses_current_ttl() {
    let token = FirstPushToken {
        token_hash: "sha256:test".to_string(),
        secret: Some("scope_fp_test".to_string()),
        owner_user_id: test_owner_id(),
        created_at_unix: 1000,
        expires_at_unix: 1000 + (60 * 60 * 24),
        used_at_unix: None,
    };

    let active = first_push_token_response(&token, 1000, None);
    assert_eq!(active.status, FirstPushTokenStatus::Active);
    assert_eq!(active.expires_at_unix, 1000 + FIRST_PUSH_TOKEN_TTL_SECS);
    assert_eq!(active.secret.as_deref(), Some("scope_fp_test"));

    let expired = first_push_token_response(&token, 1000 + FIRST_PUSH_TOKEN_TTL_SECS, None);
    assert_eq!(expired.status, FirstPushTokenStatus::Expired);
    assert!(expired.secret.is_none());
}

#[tokio::test]
async fn list_repos_route_requires_sign_in() {
    let response = router(test_state_with_jwks())
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/v1/repos")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn list_repos_route_hides_pending_repo_from_reader_member() {
    let state = test_state_with_repo();
    cache_test_jwks(&state);
    let reader_identity = ShooIdentity {
        pairwise_sub: "pairwise-reader".to_string(),
        email: Some("reader@example.com".to_string()),
        email_verified: true,
    };
    let reader_id = identity_user_id(&reader_identity);
    {
        let mut catalog = lock_catalog(&state).unwrap();
        catalog.users.insert(
            reader_id.clone(),
            UserAccount {
                id: reader_id.clone(),
                handle: "reader".to_string(),
                email: "reader@example.com".to_string(),
                email_verified: true,
                access: AccountAccess::Member,
            },
        );
        let repo = catalog.repositories.get_mut(TEST_REPO_ID).unwrap();
        repo.record.publication_state = RepoPublicationState::PendingPublish;
        repo.memberships.push(RepoMembership {
            repo_id: TEST_REPO_ID.to_string(),
            user_id: reader_id,
            role: RepoRole::Reader,
        });
    }

    let response = router(state)
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/v1/repos")
                .header(
                    AUTHORIZATION,
                    bearer_header_for("pairwise-reader", "reader@example.com"),
                )
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let body = response_json(response).await;
    assert_eq!(body.as_array().unwrap().len(), 0);
}

#[tokio::test]
async fn pending_publish_repo_session_is_owner_only() {
    let state = test_state_with_repo();
    cache_test_jwks(&state);
    {
        let mut catalog = lock_catalog(&state).unwrap();
        let repo = catalog.repositories.get_mut(TEST_REPO_ID).unwrap();
        repo.record.publication_state = RepoPublicationState::PendingPublish;
    }
    let app = router(state);

    let public_response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/v1/repos/owner/repo/session")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(public_response.status(), StatusCode::NOT_FOUND);

    let owner_response = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/v1/repos/owner/repo/session")
                .header(AUTHORIZATION, bearer_header())
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(owner_response.status(), StatusCode::OK);
    let body = response_json(owner_response).await;
    assert_eq!(body["principal_id"], test_owner_id());
    assert_eq!(body["capabilities"]["read"], true);
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
fn pending_import_review_uses_default_visibility() {
    let mut repo = test_repo(&test_owner_id());
    repo.record.publication_state = RepoPublicationState::PendingPublish;
    repo.record.default_visibility = Visibility::Private;
    repo.policy = Policy::new(Visibility::Private, repo.record.owner_user_id.clone());
    repo.pending_import = Some(pending_import_fixture(vec![
        ("README.md", "hello"),
        ("src/main.rs", "fn main() {}"),
    ]));
    let owner = Principal {
        id: repo.record.owner_user_id.clone(),
        kind: PrincipalKind::User,
    };

    let files = pending_import_files(&repo, &owner).unwrap();

    assert_eq!(files.len(), 2);
    assert!(
        files
            .iter()
            .all(|file| file.visibility == Visibility::Private)
    );
}

#[test]
fn pending_visibility_toggles_apply_before_publish() {
    let mut repo = test_repo(&test_owner_id());
    repo.record.publication_state = RepoPublicationState::PendingPublish;
    repo.pending_import = Some(pending_import_fixture(vec![("README.md", "hello")]));
    let path = ScopePath::parse("/README.md").unwrap();
    repo.policy
        .add_rule(VisibilityRule::private(path.clone(), repo_owner_ids(&repo)))
        .unwrap();
    let owner = Principal {
        id: repo.record.owner_user_id.clone(),
        kind: PrincipalKind::User,
    };

    let private_files = files_for_visibility_update(&repo, &owner).unwrap();
    assert_eq!(private_files[0].visibility, Visibility::Private);

    repo.policy.add_rule(VisibilityRule::public(path)).unwrap();
    let public_files = files_for_visibility_update(&repo, &owner).unwrap();
    assert_eq!(public_files[0].visibility, Visibility::Public);
}

#[test]
fn zero_file_publish_promotes_pending_import() {
    let mut repo = test_repo(&test_owner_id());
    repo.record.publication_state = RepoPublicationState::PendingPublish;
    repo.pending_import = Some(pending_import_fixture(Vec::new()));

    promote_pending_import(&mut repo).unwrap();

    assert_eq!(
        repo.record.publication_state,
        RepoPublicationState::Published
    );
    assert!(repo.pending_import.is_none());
    assert_eq!(repo.graph.commits.len(), 1);
    assert!(repo.graph.commits[0].changes.is_empty());
}

#[test]
fn publish_is_one_time() {
    let mut repo = test_repo(&test_owner_id());
    repo.record.publication_state = RepoPublicationState::PendingPublish;
    repo.pending_import = Some(pending_import_fixture(vec![("README.md", "hello")]));

    promote_pending_import(&mut repo).unwrap();
    let error = promote_pending_import(&mut repo).unwrap_err();

    assert_eq!(error.status, StatusCode::BAD_REQUEST);
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
                    new_content: Some("hello".to_string()),
                },
                FileChange {
                    path: ScopePath::parse("/secret.txt").unwrap(),
                    old_content: None,
                    new_content: Some("nope".to_string()),
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

    let repo_path = projection_bare_repo(&cache_root, &projection).unwrap();
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
fn repo_settings_review_pushes_default_on() {
    assert!(RepoSettings::default().review_pushes_before_applying);
    assert!(!RepoSettings::default().include_ignored_files);
}
