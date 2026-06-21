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
async fn db_metadata_route_round_trips_from_clean_database() {
    let Some(database_url) = std::env::var("SCOPE_TEST_DATABASE_URL")
        .ok()
        .filter(|value| !value.trim().is_empty())
    else {
        eprintln!("skipping DB metadata route test; SCOPE_TEST_DATABASE_URL is not set");
        return;
    };
    let metadata = crate::db::MetadataStore::connect_for_tests(database_url.clone()).unwrap();
    metadata
        .update(|catalog| {
            catalog.users.clear();
            catalog.repositories.clear();
            Ok(())
        })
        .unwrap();

    let app = router(test_state_with_metadata(metadata));
    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/repos")
                .header(AUTHORIZATION, bearer_header())
                .header(CONTENT_TYPE, "application/json")
                .body(Body::from(r#"{"name":"db-backed"}"#))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let body = response_json(response).await;
    assert_eq!(body["repo"]["id"], "owner/db-backed");

    let fresh_metadata = crate::db::MetadataStore::connect_for_tests(database_url).unwrap();
    let response = router(test_state_with_metadata(fresh_metadata))
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
    assert_eq!(body[0]["id"], "owner/db-backed");
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
async fn projection_route_returns_content_not_blob_metadata() {
    let state = test_state_with_repo();
    cache_test_jwks(&state);
    {
        let mut catalog = lock_catalog(&state).unwrap();
        catalog
            .repositories
            .insert(TEST_REPO_ID.to_string(), repo_with_readme());
    }

    let response = router(state)
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/v1/repos/owner/repo/projections")
                .header(AUTHORIZATION, bearer_header())
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let body = response_json(response).await;
    let content = &body["commits"][0]["changes"][0]["new_content"];
    assert_eq!(content, "hello");
    assert!(content["object_key"].is_null());
}

#[tokio::test]
async fn visibility_update_uses_verified_email_canonical_owner() {
    let state = test_state_with_repo();
    cache_test_jwks(&state);
    {
        let mut catalog = lock_catalog(&state).unwrap();
        catalog
            .repositories
            .insert(TEST_REPO_ID.to_string(), repo_with_readme());
    }

    let response = router(state.clone())
        .oneshot(
            Request::builder()
                .method("PATCH")
                .uri("/v1/repos/owner/repo/files/visibility")
                .header(
                    AUTHORIZATION,
                    bearer_header_for("rotated-pairwise-owner", TEST_OWNER_EMAIL),
                )
                .header(CONTENT_TYPE, "application/json")
                .body(Body::from(
                    r#"{"path":"/README.md","visibility":"Private"}"#,
                ))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let body = response_json(response).await;
    assert_eq!(body["path"], "/README.md");
    assert_eq!(body["visibility"], "Private");

    let catalog = lock_catalog(&state).unwrap();
    assert_eq!(catalog.users.len(), 1);
    let repo = catalog.repositories.get(TEST_REPO_ID).unwrap();
    let path = ScopePath::parse("/README.md").unwrap();
    assert_eq!(repo.policy.effective_visibility(&path), Visibility::Private);
}

#[tokio::test]
async fn settings_update_uses_verified_email_canonical_owner() {
    let state = test_state_with_repo();
    cache_test_jwks(&state);

    let response = router(state.clone())
        .oneshot(
            Request::builder()
                .method("PATCH")
                .uri("/v1/repos/owner/repo/settings")
                .header(
                    AUTHORIZATION,
                    bearer_header_for("rotated-pairwise-owner", TEST_OWNER_EMAIL),
                )
                .header(CONTENT_TYPE, "application/json")
                .body(Body::from(
                    r#"{"include_ignored_files":true,"review_pushes_before_applying":false}"#,
                ))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let body = response_json(response).await;
    assert_eq!(body["include_ignored_files"], true);
    assert_eq!(body["review_pushes_before_applying"], false);

    let catalog = lock_catalog(&state).unwrap();
    assert_eq!(catalog.users.len(), 1);
    let repo = catalog.repositories.get(TEST_REPO_ID).unwrap();
    assert!(repo.settings.include_ignored_files);
    assert!(!repo.settings.review_pushes_before_applying);
}

#[tokio::test]
async fn delete_repo_route_requires_owner_and_removes_storage() {
    let state = test_state_with_repo();
    cache_test_jwks(&state);
    {
        let mut catalog = lock_catalog(&state).unwrap();
        let mut repo = repo_with_readme();
        repo.graph.commits[0].changes[0].new_content = Some(source_blob("delete route readme"));
        catalog.repositories.insert(TEST_REPO_ID.to_string(), repo);
    }
    let source_keys =
        repo_source_blobs(&find_repo(&state, TEST_REPO_OWNER, TEST_REPO_NAME).unwrap())
            .into_iter()
            .map(|blob| blob.object_key)
            .collect::<Vec<_>>();
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
    for key in source_keys {
        assert!(!MemoryObjectStore::new().contains_key(&key));
    }
    assert!(!owner_repo.exists());
    assert!(!staged_repo.exists());
    assert!(!rx_repo.exists());
}

#[tokio::test]
async fn delete_repo_route_records_pending_cleanup_when_bucket_delete_fails() {
    let mut state = test_state_with_repo();
    cache_test_jwks(&state);
    {
        let mut catalog = lock_catalog(&state).unwrap();
        catalog
            .repositories
            .insert(TEST_REPO_ID.to_string(), repo_with_readme());
    }
    state.object_store = Arc::new(DeleteFailsObjectStore);
    let owner_repo = owner_git_repo_path(&state, TEST_REPO_OWNER, TEST_REPO_NAME);
    let staged_repo = staged_git_repo_path(&state, TEST_REPO_OWNER, TEST_REPO_NAME);
    let rx_repo = git_repo_storage_root(&state).join("git-rx").join(format!(
        "{}-delete-fails.git",
        receive_pack_staging_repo_prefix(TEST_REPO_OWNER, TEST_REPO_NAME)
    ));
    fs::create_dir_all(&owner_repo).unwrap();
    fs::create_dir_all(&staged_repo).unwrap();
    fs::create_dir_all(&rx_repo).unwrap();
    let app = router(state.clone());

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
    assert!(find_repo(&state, TEST_REPO_OWNER, TEST_REPO_NAME).is_err());
    assert!(
        !lock_catalog(&state)
            .unwrap()
            .pending_source_blob_deletions
            .is_empty()
    );
    assert!(!owner_repo.exists());
    assert!(!staged_repo.exists());
    assert!(!rx_repo.exists());
}

struct DeleteFailsObjectStore;

impl crate::object_store::ObjectStore for DeleteFailsObjectStore {
    fn put(&self, _key: &str, _bytes: &[u8]) -> Result<(), crate::error::ApiError> {
        Ok(())
    }

    fn get(&self, key: &str) -> Result<Vec<u8>, crate::error::ApiError> {
        Err(crate::error::ApiError::not_found(format!(
            "object {key} not found"
        )))
    }

    fn delete(&self, _key: &str) -> Result<(), crate::error::ApiError> {
        Err(crate::error::ApiError::service_unavailable("delete failed"))
    }
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
