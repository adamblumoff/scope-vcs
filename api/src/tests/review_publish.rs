use super::*;

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

#[tokio::test]
async fn pending_visibility_toggle_does_not_create_public_projection_history() {
    let state = test_state_with_repo();
    cache_test_jwks(&state);
    {
        let mut catalog = lock_catalog(&state).unwrap();
        let repo = catalog.repositories.get_mut(TEST_REPO_ID).unwrap();
        repo.record.publication_state = RepoPublicationState::PendingPublish;
        repo.record.default_visibility = Visibility::Public;
        repo.policy = Policy::new(Visibility::Public, repo.record.owner_user_id.clone());
        repo.graph.commits.clear();
        repo.pending_import = Some(pending_import_fixture(vec![
            ("README.md", "private before publish"),
            ("public.md", "public before publish"),
        ]));
    }
    let app = router(state.clone());

    let response = app
        .oneshot(
            Request::builder()
                .method("PATCH")
                .uri("/v1/repos/owner/repo/files/visibility")
                .header(AUTHORIZATION, bearer_header())
                .header(CONTENT_TYPE, "application/json")
                .body(Body::from(
                    r#"{"paths":["/README.md"],"visibility":"Private"}"#,
                ))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    {
        let mut catalog = lock_catalog(&state).unwrap();
        let repo = catalog.repositories.get_mut(TEST_REPO_ID).unwrap();
        assert!(repo.graph.commits.is_empty());
        promote_pending_import(repo).unwrap();
    }
    let repo = find_repo(&state, TEST_REPO_OWNER, TEST_REPO_NAME).unwrap();
    let projection = project_graph(&repo.policy, &repo.graph, &Principal::public());

    assert!(
        projection
            .commits
            .iter()
            .flat_map(|commit| commit.changes.iter())
            .all(|change| change.path.as_str() != "/README.md")
    );
    assert!(
        projection
            .commits
            .iter()
            .flat_map(|commit| commit.changes.iter())
            .any(|change| change.path.as_str() == "/public.md" && change.new_content.is_some())
    );
}

#[tokio::test]
async fn owner_can_preview_pending_import_public_projection_before_publish() {
    let state = test_state_with_repo();
    cache_test_jwks(&state);
    {
        let mut catalog = lock_catalog(&state).unwrap();
        let repo = catalog.repositories.get_mut(TEST_REPO_ID).unwrap();
        repo.record.publication_state = RepoPublicationState::PendingPublish;
        repo.record.default_visibility = Visibility::Public;
        repo.policy = Policy::new(Visibility::Public, repo.record.owner_user_id.clone());
        repo.policy
            .add_rule(VisibilityRule::private(
                ScopePath::parse("/secret.txt").unwrap(),
                repo_owner_ids(repo),
            ))
            .unwrap();
        repo.pending_import = Some(pending_import_fixture(vec![
            ("README.md", "hello"),
            ("secret.txt", "private"),
        ]));
    }

    let response = router(state.clone())
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/v1/repos/owner/repo/projection-preview?audience=public&source=review")
                .header(AUTHORIZATION, bearer_header())
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let body = response_json(response).await;
    assert_eq!(body["audience"], "public");
    assert_eq!(body["source"], "review");
    assert_eq!(body["summary"]["visible_files"], 1);
    assert_eq!(body["summary"]["hidden_files"], 1);
    assert_eq!(body["summary"]["synthetic_commits"], 1);
    assert_eq!(body["files"][0]["path"], "/README.md");
    assert!(
        body["files"]
            .as_array()
            .unwrap()
            .iter()
            .all(|file| file["path"] != "/secret.txt")
    );

    let repo = find_repo(&state, TEST_REPO_OWNER, TEST_REPO_NAME).unwrap();
    assert_eq!(
        repo.record.publication_state,
        RepoPublicationState::PendingPublish
    );
    assert!(repo.pending_import.is_some());
    assert!(repo.graph.commits.is_empty());
}

#[tokio::test]
async fn public_cannot_preview_pending_import_review() {
    let state = test_state_with_repo();
    {
        let mut catalog = lock_catalog(&state).unwrap();
        let repo = catalog.repositories.get_mut(TEST_REPO_ID).unwrap();
        repo.record.publication_state = RepoPublicationState::PendingPublish;
        repo.pending_import = Some(pending_import_fixture(vec![("README.md", "hello")]));
    }

    let response = router(state)
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/v1/repos/owner/repo/projection-preview?audience=public&source=review")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn owner_can_preview_staged_update_public_projection_before_apply() {
    let state = test_state_with_repo();
    cache_test_jwks(&state);
    {
        let mut catalog = lock_catalog(&state).unwrap();
        let repo = repo_with_readme();
        catalog.repositories.insert(TEST_REPO_ID.to_string(), repo);
        let repo = catalog.repositories.get_mut(TEST_REPO_ID).unwrap();
        repo.staged_update = Some(StagedRepoUpdate {
            id: "staged_push_1".to_string(),
            branch: format!("refs/heads/{DEFAULT_GIT_BRANCH}"),
            base_live_commit_id: repo.graph.commits.last().map(|commit| commit.id.clone()),
            author_id: repo.record.owner_user_id.clone(),
            message: "preview staged update".to_string(),
            git_snapshot: source_blob("preview staged git snapshot"),
            changes: vec![
                StagedFileChange {
                    path: ScopePath::parse("/docs/guide.md").unwrap(),
                    old_content: None,
                    new_content: Some(source_blob("public docs")),
                    visibility: Visibility::Public,
                    kind: StagedFileChangeKind::Added,
                },
                StagedFileChange {
                    path: ScopePath::parse("/secret.txt").unwrap(),
                    old_content: None,
                    new_content: Some(source_blob("private staged content")),
                    visibility: Visibility::Private,
                    kind: StagedFileChangeKind::Added,
                },
            ],
        });
    }

    let response = router(state.clone())
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/v1/repos/owner/repo/projection-preview?audience=public&source=review")
                .header(AUTHORIZATION, bearer_header())
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let body = response_json(response).await;
    let files = body["files"].as_array().unwrap();
    assert!(files.iter().any(|file| file["path"] == "/README.md"));
    assert!(files.iter().any(|file| file["path"] == "/docs/guide.md"));
    assert!(files.iter().all(|file| file["path"] != "/secret.txt"));
    assert_eq!(body["summary"]["hidden_files"], 1);

    let repo = find_repo(&state, TEST_REPO_OWNER, TEST_REPO_NAME).unwrap();
    assert!(repo.staged_update.is_some());
    assert_eq!(repo.graph.commits.len(), 1);
}

#[test]
fn zero_file_publish_promotes_pending_import() {
    let mut repo = test_repo(&test_owner_id());
    repo.record.publication_state = RepoPublicationState::PendingPublish;
    repo.pending_import = Some(pending_import_fixture(Vec::new()));
    repo.first_push_token = Some(FirstPushToken {
        token_hash: first_push_token_hash("scope_fp_test"),
        secret: Some("scope_fp_test".to_string()),
        owner_user_id: repo.record.owner_user_id.clone(),
        created_at_unix: unix_now(),
        expires_at_unix: unix_now() + FIRST_PUSH_TOKEN_TTL_SECS,
        used_at_unix: Some(unix_now()),
    });
    repo.git_push_token = Some(GitPushToken {
        token_hash: git_push_token_hash("scope_git_test"),
        owner_user_id: repo.record.owner_user_id.clone(),
        created_at_unix: unix_now(),
    });

    promote_pending_import(&mut repo).unwrap();

    assert_eq!(
        repo.record.publication_state,
        RepoPublicationState::Published
    );
    assert!(repo.pending_import.is_none());
    assert!(repo.first_push_token.is_none());
    assert!(repo.git_push_token.is_none());
    assert_eq!(repo.graph.commits.len(), 1);
    assert!(repo.graph.commits[0].changes.is_empty());
}

#[tokio::test]
async fn publish_uses_verified_email_canonical_owner() {
    let state = test_state_with_repo();
    cache_test_jwks(&state);
    {
        let mut catalog = lock_catalog(&state).unwrap();
        let repo = catalog.repositories.get_mut(TEST_REPO_ID).unwrap();
        repo.record.publication_state = RepoPublicationState::PendingPublish;
        repo.pending_import = Some(pending_import_fixture(vec![("README.md", "hello")]));
    }

    let response = router(state.clone())
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/repos/owner/repo/publish")
                .header(
                    AUTHORIZATION,
                    bearer_header_for("rotated-pairwise-owner", TEST_OWNER_EMAIL),
                )
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let body = response_json(response).await;
    assert_eq!(body["publication_state"], "Published");
    assert_eq!(body["role"], "Owner");

    let catalog = lock_catalog(&state).unwrap();
    assert_eq!(catalog.users.len(), 1);
    let repo = catalog.repositories.get(TEST_REPO_ID).unwrap();
    assert_eq!(
        repo.record.publication_state,
        RepoPublicationState::Published
    );
    assert!(repo.pending_import.is_none());
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
fn repo_settings_review_pushes_default_on() {
    assert!(RepoSettings::default().review_pushes_before_applying);
    assert!(!RepoSettings::default().include_ignored_files);
}

#[test]
fn rejecting_staged_update_deletes_unreferenced_bucket_objects() {
    let state = test_state_with_repo();
    let rejected_blob = source_blob("rejected private content");
    let rejected_key = rejected_blob.object_key.clone();
    {
        let mut repo = repo_with_readme();
        repo.staged_update = Some(StagedRepoUpdate {
            id: "staged_push_1".to_string(),
            branch: format!("refs/heads/{DEFAULT_GIT_BRANCH}"),
            base_live_commit_id: repo.graph.commits.last().map(|commit| commit.id.clone()),
            author_id: repo.record.owner_user_id.clone(),
            message: "reject me".to_string(),
            git_snapshot: source_blob("rejected staged git snapshot"),
            changes: vec![StagedFileChange {
                path: ScopePath::parse("/private.txt").unwrap(),
                old_content: None,
                new_content: Some(rejected_blob),
                visibility: Visibility::Private,
                kind: StagedFileChangeKind::Added,
            }],
        });
        let mut catalog = lock_catalog(&state).unwrap();
        catalog.repositories.insert(TEST_REPO_ID.to_string(), repo);
    }

    reject_staged_update_as_owner(&state).unwrap();

    assert!(!MemoryObjectStore::new().contains_key(&rejected_key));
}

#[test]
fn rejecting_staged_update_records_pending_cleanup_when_bucket_delete_fails() {
    let mut state = test_state_with_repo();
    state.object_store = Arc::new(DeleteFailsObjectStore);
    let rejected_blob = source_blob("rejected cleanup failure content");
    let rejected_key = rejected_blob.object_key.clone();
    {
        let mut repo = repo_with_readme();
        repo.staged_update = Some(StagedRepoUpdate {
            id: "staged_push_1".to_string(),
            branch: format!("refs/heads/{DEFAULT_GIT_BRANCH}"),
            base_live_commit_id: repo.graph.commits.last().map(|commit| commit.id.clone()),
            author_id: repo.record.owner_user_id.clone(),
            message: "reject me".to_string(),
            git_snapshot: source_blob("rejected cleanup failure git snapshot"),
            changes: vec![StagedFileChange {
                path: ScopePath::parse("/private.txt").unwrap(),
                old_content: None,
                new_content: Some(rejected_blob),
                visibility: Visibility::Private,
                kind: StagedFileChangeKind::Added,
            }],
        });
        let mut catalog = lock_catalog(&state).unwrap();
        catalog.repositories.insert(TEST_REPO_ID.to_string(), repo);
    }

    reject_staged_update_as_owner(&state).unwrap();

    let repo = find_repo(&state, TEST_REPO_OWNER, TEST_REPO_NAME).unwrap();
    assert!(repo.staged_update.is_none());
    assert!(MemoryObjectStore::new().contains_key(&rejected_key));
    assert!(
        !lock_catalog(&state)
            .unwrap()
            .pending_source_blob_deletions
            .is_empty()
    );
}

#[test]
fn rejecting_staged_update_does_not_cleanup_when_metadata_persist_fails() {
    let state = test_state_with_repo();
    let rejected_blob = source_blob("rejected persist failure content");
    let rejected_key = rejected_blob.object_key.clone();
    {
        let mut repo = repo_with_readme();
        repo.staged_update = Some(StagedRepoUpdate {
            id: "staged_push_1".to_string(),
            branch: format!("refs/heads/{DEFAULT_GIT_BRANCH}"),
            base_live_commit_id: repo.graph.commits.last().map(|commit| commit.id.clone()),
            author_id: repo.record.owner_user_id.clone(),
            message: "reject me".to_string(),
            git_snapshot: source_blob("rejected persist failure git snapshot"),
            changes: vec![StagedFileChange {
                path: ScopePath::parse("/private.txt").unwrap(),
                old_content: None,
                new_content: Some(rejected_blob),
                visibility: Visibility::Private,
                kind: StagedFileChangeKind::Added,
            }],
        });
        let mut catalog = lock_catalog(&state).unwrap();
        catalog.repositories.insert(TEST_REPO_ID.to_string(), repo);
    }
    state.metadata.fail_next_persist_for_tests().unwrap();

    let error = reject_staged_update_as_owner(&state).unwrap_err();

    assert_eq!(error.status, StatusCode::INTERNAL_SERVER_ERROR);
    let repo = find_repo(&state, TEST_REPO_OWNER, TEST_REPO_NAME).unwrap();
    assert!(repo.staged_update.is_some());
    assert!(MemoryObjectStore::new().contains_key(&rejected_key));
    assert!(
        lock_catalog(&state)
            .unwrap()
            .pending_source_blob_deletions
            .is_empty()
    );
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
