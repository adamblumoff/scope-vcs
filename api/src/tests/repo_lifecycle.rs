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
    assert_eq!(
        body["init"]["git_remote_url"],
        "http://localhost:8080/git/owner/scope_app"
    );
    assert_eq!(body["init"]["remote_name"], "scope");
    assert_eq!(body["init"]["push_branch"], DEFAULT_GIT_BRANCH);
    assert_eq!(
        body["init"]["review_url"],
        "http://localhost:3000/repos/owner/scope_app/review"
    );
    let secret = body["init"]["token"]["secret"].as_str().unwrap();
    assert!(secret.starts_with("scope_fp_"));
    assert_eq!(body["init"]["token"]["status"], "Active");
    let push_secret = body["init"]["push_token"]["secret"].as_str().unwrap();
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
    assert_eq!(body[0]["lifecycle_state"], "PendingFirstPush");
    assert_eq!(body[0]["role"], "Owner");

    let catalog = lock_catalog(&state).unwrap();
    assert_eq!(catalog.users.len(), 1);
    assert_eq!(catalog.repositories.len(), 1);
    let repo = catalog.repositories.get("owner/scope_app").unwrap();
    let token = repo.first_push_token.as_ref().unwrap();
    assert_ne!(token.token_hash, secret);
    assert!(token.secret.is_none());
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
    let Some(test_db) = crate::db::TestDatabaseTarget::from_env().unwrap() else {
        eprintln!("skipping DB metadata route test; SCOPE_TEST_DATABASE_URL is not set");
        return;
    };
    let metadata = crate::db::MetadataStore::connect_fresh_for_tests(&test_db).unwrap();

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
    let secret = body["init"]["token"]["secret"].as_str().unwrap();
    let push_secret = body["init"]["push_token"]["secret"].as_str().unwrap();

    let fresh_metadata = crate::db::MetadataStore::connect_for_tests(&test_db).unwrap();
    let row_repo = fresh_metadata
        .repository(TEST_REPO_OWNER, "db-backed")
        .unwrap()
        .expect("created repo loads from row store");
    let token = row_repo.first_push_token.as_ref().unwrap();
    assert_ne!(token.token_hash, secret);
    assert!(token.secret.is_none());
    let push_token = row_repo.git_push_token.as_ref().unwrap();
    assert_ne!(push_token.token_hash, push_secret);

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
    assert_eq!(body.as_array().unwrap().len(), 0);
}

#[test]
fn db_metadata_store_round_trips_repo_metadata() {
    let Some(test_db) = crate::db::TestDatabaseTarget::from_env().unwrap() else {
        eprintln!("skipping DB metadata store test; SCOPE_TEST_DATABASE_URL is not set");
        return;
    };
    let owner_id = test_owner_id();
    let (_, first_push_token) = generate_first_push_token(&owner_id).unwrap();
    let (_, git_push_token) = generate_git_push_token(&owner_id).unwrap();
    let owner = UserAccount {
        id: owner_id.clone(),
        handle: TEST_REPO_OWNER.to_string(),
        email: TEST_OWNER_EMAIL.to_string(),
        email_verified: true,
        access: AccountAccess::Member,
    };
    let mut repo = repo_with_readme();
    let private_path = ScopePath::parse("/secret.txt").unwrap();
    repo.first_push_token = Some(first_push_token);
    repo.git_push_token = Some(git_push_token);
    repo.policy
        .add_rule(VisibilityRule::private(
            private_path.clone(),
            vec![owner_id.clone()],
        ))
        .unwrap();
    repo.pending_import = Some(pending_import_fixture(vec![("imported.txt", "imported")]));
    repo.git_snapshot = Some(source_blob("live git snapshot"));
    repo.staged_update = Some(StagedRepoUpdate {
        id: "stage-1".to_string(),
        branch: "refs/heads/main".to_string(),
        base_live_commit_id: Some("rv1".to_string()),
        author_id: owner_id.clone(),
        message: "stage update".to_string(),
        git_snapshot: source_blob("staged git snapshot"),
        changes: vec![StagedFileChange {
            path: ScopePath::parse("/README.md").unwrap(),
            old_content: repo.graph.commits[0].changes[0].new_content.clone(),
            new_content: Some(source_blob("staged readme")),
            line_diff: LineDiff::default(),
            visibility: Visibility::Public,
            kind: StagedFileChangeKind::Modified,
        }],
    });
    let pending_deletions = vec![source_blob("delete after retry")];
    let expected_pending_import = repo.pending_import.clone();
    let expected_git_snapshot = repo.git_snapshot.clone();
    let expected_staged_update = repo.staged_update.clone();
    let expected_graph = repo.graph.clone();
    let expected_pending_deletions = pending_deletions.clone();

    let metadata = crate::db::MetadataStore::connect_fresh_for_tests(&test_db).unwrap();
    metadata
        .update(move |catalog| {
            catalog.users.insert(owner.id.clone(), owner);
            catalog.repositories.insert(repo.record.id.clone(), repo);
            catalog.pending_source_blob_deletions = pending_deletions;
            Ok(())
        })
        .unwrap();

    let fresh_metadata = crate::db::MetadataStore::connect_for_tests(&test_db).unwrap();
    let row_repo = fresh_metadata
        .repository(TEST_REPO_OWNER, TEST_REPO_NAME)
        .unwrap()
        .expect("row repo loads");
    assert_eq!(row_repo.graph, expected_graph);
    assert_eq!(row_repo.pending_import, expected_pending_import);
    let row_repos = fresh_metadata.repositories_for_user(&owner_id).unwrap();
    assert_eq!(row_repos.len(), 1);
    assert_eq!(row_repos[0].record.id, TEST_REPO_ID);

    let updated_settings = RepoSettings {
        include_ignored_files: true,
        review_pushes_before_applying: false,
    };
    assert_eq!(
        fresh_metadata
            .update_repo_settings(
                TEST_REPO_OWNER,
                TEST_REPO_NAME,
                &owner_id,
                updated_settings,
                Visibility::Private,
            )
            .unwrap()
            .settings,
        updated_settings
    );
    let row_repo = fresh_metadata
        .repository(TEST_REPO_OWNER, TEST_REPO_NAME)
        .unwrap()
        .expect("row repo loads after settings update");
    assert_eq!(row_repo.settings, updated_settings);
    assert_eq!(row_repo.record.default_visibility, Visibility::Private);
    assert_eq!(
        row_repo
            .policy
            .effective_visibility(&ScopePath::parse("/new.ts").unwrap()),
        Visibility::Private
    );
    assert_eq!(
        row_repo
            .policy
            .effective_visibility(&ScopePath::parse("/README.md").unwrap()),
        Visibility::Public
    );

    fresh_metadata
        .read(move |catalog| {
            let repo = catalog.repositories.get(TEST_REPO_ID).unwrap();
            assert_eq!(repo.graph, expected_graph);
            assert_eq!(
                repo.policy.effective_visibility(&private_path),
                Visibility::Private
            );
            assert_eq!(repo.pending_import, expected_pending_import);
            assert_eq!(repo.git_snapshot, expected_git_snapshot);
            assert_eq!(repo.staged_update, expected_staged_update);
            assert_eq!(
                catalog.pending_source_blob_deletions,
                expected_pending_deletions
            );
            Ok(())
        })
        .unwrap();

    let readme_path = ScopePath::parse("/README.md").unwrap();
    let updated_repo = fresh_metadata
        .update_repo_file_visibility(
            TEST_REPO_OWNER,
            TEST_REPO_NAME,
            &owner_id,
            vec![readme_path.clone()],
            Visibility::Private,
        )
        .unwrap();
    assert_eq!(
        updated_repo.policy.effective_visibility(&readme_path),
        Visibility::Private
    );
    assert!(
        updated_repo
            .graph
            .commits
            .iter()
            .any(|commit| commit.id.starts_with("rv_visibility_"))
    );
    let row_repo = fresh_metadata
        .repository(TEST_REPO_OWNER, TEST_REPO_NAME)
        .unwrap()
        .expect("row repo loads after visibility update");
    assert_eq!(
        row_repo.policy.effective_visibility(&readme_path),
        Visibility::Private
    );
    assert_eq!(row_repo.graph, updated_repo.graph);
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
async fn visibility_update_rejects_different_clerk_user_with_same_email() {
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
                    bearer_header_for("user_rotated_owner", TEST_OWNER_EMAIL),
                )
                .header(CONTENT_TYPE, "application/json")
                .body(Body::from(
                    r#"{"paths":["/README.md"],"visibility":"Private"}"#,
                ))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::FORBIDDEN);

    let catalog = lock_catalog(&state).unwrap();
    assert_eq!(catalog.users.len(), 2);
    let repo = catalog.repositories.get(TEST_REPO_ID).unwrap();
    let path = ScopePath::parse("/README.md").unwrap();
    assert_eq!(repo.policy.effective_visibility(&path), Visibility::Public);
}

#[tokio::test]
async fn visibility_update_batches_multiple_paths() {
    let state = test_state_with_repo();
    cache_test_jwks(&state);
    {
        let mut repo = repo_with_readme();
        repo.graph.commits[0].changes.push(FileChange {
            visibility: Visibility::Public,
            path: ScopePath::parse("/src/app.ts").unwrap(),
            old_content: None,
            new_content: Some(source_blob("app")),
        });
        let mut catalog = lock_catalog(&state).unwrap();
        catalog.repositories.insert(TEST_REPO_ID.to_string(), repo);
    }

    let response = router(state.clone())
        .oneshot(
            Request::builder()
                .method("PATCH")
                .uri("/v1/repos/owner/repo/files/visibility")
                .header(AUTHORIZATION, bearer_header())
                .header(CONTENT_TYPE, "application/json")
                .body(Body::from(
                    r#"{"paths":["/README.md","/src/app.ts"],"visibility":"Private"}"#,
                ))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let body = response_json(response).await;
    assert_eq!(body.as_array().unwrap().len(), 2);
    assert!(
        body.as_array()
            .unwrap()
            .iter()
            .all(|file| file["visibility"] == "Private")
    );

    let catalog = lock_catalog(&state).unwrap();
    let repo = catalog.repositories.get(TEST_REPO_ID).unwrap();
    assert_eq!(
        repo.policy
            .effective_visibility(&ScopePath::parse("/README.md").unwrap()),
        Visibility::Private
    );
    assert_eq!(
        repo.policy
            .effective_visibility(&ScopePath::parse("/src/app.ts").unwrap()),
        Visibility::Private
    );
}

#[tokio::test]
async fn settings_update_rejects_different_clerk_user_with_same_email() {
    let state = test_state_with_repo();
    cache_test_jwks(&state);

    let response = router(state.clone())
        .oneshot(
            Request::builder()
                .method("PATCH")
                .uri("/v1/repos/owner/repo/settings")
                .header(
                    AUTHORIZATION,
                    bearer_header_for("user_rotated_owner", TEST_OWNER_EMAIL),
                )
                .header(CONTENT_TYPE, "application/json")
                .body(Body::from(
                    r#"{"default_new_file_visibility":"Private","review_pushes_before_applying":false}"#,
                ))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::FORBIDDEN);

    let catalog = lock_catalog(&state).unwrap();
    assert_eq!(catalog.users.len(), 2);
    let repo = catalog.repositories.get(TEST_REPO_ID).unwrap();
    assert!(!repo.settings.include_ignored_files);
    assert!(repo.settings.review_pushes_before_applying);
    assert_eq!(repo.record.default_visibility, Visibility::Public);
    assert_eq!(
        repo.policy
            .effective_visibility(&ScopePath::parse("/new.ts").unwrap()),
        Visibility::Public
    );
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
    let reader_identity = ClerkIdentity {
        user_id: "user_reader".to_string(),
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
                    bearer_header_for("user_reader", "reader@example.com"),
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
