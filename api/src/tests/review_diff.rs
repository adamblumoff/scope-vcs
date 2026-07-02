use super::*;

#[tokio::test]
async fn owner_can_load_pending_import_file_diff() {
    let state = test_state_with_repo();
    cache_test_jwks(&state);
    {
        let mut catalog = lock_catalog(&state).unwrap();
        let repo = catalog.repositories.get_mut(TEST_REPO_ID).unwrap();
        repo.record.publication_state = RepoPublicationState::Unpublished;
        repo.pending_import = Some(pending_import_fixture(vec![
            ("README.md", "hello from import"),
            ("src/main.rs", "fn main() {}"),
        ]));
    }

    let response = router(state)
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/v1/repos/owner/repo/review/file-diff?path=README.md")
                .header(AUTHORIZATION, bearer_header())
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let body = response_json(response).await;
    assert_eq!(body["path"], "/README.md");
    assert_eq!(body["kind"], "Added");
    assert_eq!(body["old_content"], serde_json::Value::Null);
    assert_text_content(&body["new_content"], "hello from import");
}

#[tokio::test]
async fn pending_import_binary_file_diff_returns_binary_metadata() {
    let state = test_state_with_repo();
    cache_test_jwks(&state);
    let binary = b"\x89PNG\r\n\x1a\nreview-binary\0\xff";
    let blob = source_blob_from_bytes(binary);
    let oid = blob.git_oid.clone();
    let size_bytes = blob.size_bytes;
    {
        let mut catalog = lock_catalog(&state).unwrap();
        let repo = catalog.repositories.get_mut(TEST_REPO_ID).unwrap();
        repo.record.publication_state = RepoPublicationState::Unpublished;
        repo.pending_import = Some(PendingImport {
            default_branch: DEFAULT_GIT_BRANCH.to_string(),
            head_oid: "1111111111111111111111111111111111111111".to_string(),
            tree_oid: "2222222222222222222222222222222222222222".to_string(),
            imported_at_unix: unix_now(),
            git_snapshot: source_blob("test git snapshot"),
            files: vec![PendingImportFile {
                path: "image.png".to_string(),
                mode: DEFAULT_GIT_FILE_MODE.to_string(),
                oid: oid.clone(),
                blob,
            }],
        });
    }

    let response = router(state)
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/v1/repos/owner/repo/review/file-diff?path=image.png")
                .header(AUTHORIZATION, bearer_header())
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let body = response_json(response).await;
    assert_eq!(body["path"], "/image.png");
    assert_eq!(body["kind"], "Added");
    assert_eq!(body["old_content"], serde_json::Value::Null);
    assert_binary_content(&body["new_content"], &oid, size_bytes);
}

#[tokio::test]
async fn large_binary_file_diff_uses_metadata_without_fetching_blob() {
    let state = test_state_with_repo();
    cache_test_jwks(&state);
    let oid = "3333333333333333333333333333333333333333".to_string();
    let size_bytes = 1024 * 1024 + 1;
    let blob = crate::domain::store::SourceBlob {
        object_key: "objects/missing-large-binary".to_string(),
        sha256: "missing-large-binary-sha".to_string(),
        git_oid: oid.clone(),
        git_file_mode: DEFAULT_GIT_FILE_MODE.to_string(),
        size_bytes,
    };
    {
        let mut catalog = lock_catalog(&state).unwrap();
        let repo = catalog.repositories.get_mut(TEST_REPO_ID).unwrap();
        repo.record.publication_state = RepoPublicationState::Unpublished;
        repo.pending_import = Some(PendingImport {
            default_branch: DEFAULT_GIT_BRANCH.to_string(),
            head_oid: "1111111111111111111111111111111111111111".to_string(),
            tree_oid: "2222222222222222222222222222222222222222".to_string(),
            imported_at_unix: unix_now(),
            git_snapshot: source_blob("test git snapshot"),
            files: vec![PendingImportFile {
                path: "large.bin".to_string(),
                mode: DEFAULT_GIT_FILE_MODE.to_string(),
                oid: oid.clone(),
                blob,
            }],
        });
    }

    let app = router(state);
    let diff_response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/v1/repos/owner/repo/review/file-diff?path=large.bin")
                .header(AUTHORIZATION, bearer_header())
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(diff_response.status(), StatusCode::OK);
    let diff = response_json(diff_response).await;
    assert_binary_content(&diff["new_content"], &oid, size_bytes);

    let review_response = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/v1/repos/owner/repo/pending-import")
                .header(AUTHORIZATION, bearer_header())
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(review_response.status(), StatusCode::OK);
    let review = response_json(review_response).await;
    assert_eq!(review["line_diff"]["additions"], 0);
    assert_eq!(review["line_diff"]["deletions"], 0);
}

#[tokio::test]
async fn pending_import_review_includes_total_line_diff() {
    let state = test_state_with_repo();
    cache_test_jwks(&state);
    {
        let mut catalog = lock_catalog(&state).unwrap();
        let repo = catalog.repositories.get_mut(TEST_REPO_ID).unwrap();
        repo.record.publication_state = RepoPublicationState::Unpublished;
        repo.pending_import = Some(pending_import_fixture(vec![
            ("README.md", "hello\nfrom import\n"),
            ("src/main.rs", "fn main() {}"),
        ]));
    }

    let response = router(state)
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/v1/repos/owner/repo/pending-import")
                .header(AUTHORIZATION, bearer_header())
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let body = response_json(response).await;
    assert_eq!(body["line_diff"]["deletions"], 0);
    assert_eq!(body["line_diff"]["additions"], 3);
}

#[tokio::test]
async fn owner_can_load_staged_update_file_diff() {
    let state = test_state_with_repo();
    cache_test_jwks(&state);
    {
        let mut repo = repo_with_readme();
        stage_receive_pack_update(
            &mut repo,
            receive_pack_update(vec![("/README.md", Some("updated readme"))]),
        )
        .unwrap();

        let mut catalog = lock_catalog(&state).unwrap();
        catalog.repositories.insert(TEST_REPO_ID.to_string(), repo);
    }

    let response = router(state)
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/v1/repos/owner/repo/review/file-diff?path=/README.md")
                .header(AUTHORIZATION, bearer_header())
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let body = response_json(response).await;
    assert_eq!(body["path"], "/README.md");
    assert_eq!(body["kind"], "Modified");
    assert_text_content(&body["old_content"], "hello");
    assert_text_content(&body["new_content"], "updated readme");
}

#[tokio::test]
async fn staged_update_review_includes_total_line_diff() {
    let state = test_state_with_repo();
    cache_test_jwks(&state);
    {
        let mut repo = repo_with_readme();
        stage_receive_pack_update(
            &mut repo,
            receive_pack_update(vec![
                ("/README.md", Some("hello\nnew line")),
                ("/docs/guide.md", Some("first\nsecond")),
            ]),
        )
        .unwrap();

        let mut catalog = lock_catalog(&state).unwrap();
        catalog.repositories.insert(TEST_REPO_ID.to_string(), repo);
    }

    let response = router(state)
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/v1/repos/owner/repo/staged-update")
                .header(AUTHORIZATION, bearer_header())
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let body = response_json(response).await;
    assert_eq!(body["line_diff"]["deletions"], 0);
    assert_eq!(body["line_diff"]["additions"], 3);
}

#[tokio::test]
async fn staged_update_review_counts_separate_line_diff_hunks() {
    let state = test_state_with_repo();
    cache_test_jwks(&state);
    {
        let mut repo = repo_with_readme();
        repo.graph.commits[0].changes[0].new_content =
            Some(source_blob("one\nold-a\nsame\nold-b\nlast"));
        stage_receive_pack_update(
            &mut repo,
            receive_pack_update(vec![("/README.md", Some("one\nnew-a\nsame\nnew-b\nlast"))]),
        )
        .unwrap();

        let mut catalog = lock_catalog(&state).unwrap();
        catalog.repositories.insert(TEST_REPO_ID.to_string(), repo);
    }

    let response = router(state)
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/v1/repos/owner/repo/staged-update")
                .header(AUTHORIZATION, bearer_header())
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let body = response_json(response).await;
    assert_eq!(body["line_diff"]["deletions"], 2);
    assert_eq!(body["line_diff"]["additions"], 2);
}

#[tokio::test]
async fn rejecting_staged_update_returns_line_diff_before_cleanup() {
    let state = test_state_with_repo();
    cache_test_jwks(&state);
    let rejected_blob_key = {
        let mut repo = repo_with_readme();
        stage_receive_pack_update(
            &mut repo,
            receive_pack_update(vec![("/README.md", Some("hello\nrejected line"))]),
        )
        .unwrap();
        let rejected_blob_key = repo
            .staged_update
            .as_ref()
            .unwrap()
            .changes
            .first()
            .unwrap()
            .new_content
            .as_ref()
            .unwrap()
            .object_key
            .clone();

        let mut catalog = lock_catalog(&state).unwrap();
        catalog.repositories.insert(TEST_REPO_ID.to_string(), repo);
        rejected_blob_key
    };

    let response = router(state)
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/repos/owner/repo/staged-update/reject")
                .header(AUTHORIZATION, bearer_header())
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let body = response_json(response).await;
    assert_eq!(body["line_diff"]["deletions"], 0);
    assert_eq!(body["line_diff"]["additions"], 1);
    assert!(!MemoryObjectStore::new().contains_key(&rejected_blob_key));
}

#[tokio::test]
async fn apply_staged_update_blob_read_failure_does_not_commit_metadata() {
    let mut state = test_state_with_repo();
    cache_test_jwks(&state);
    {
        let mut repo = repo_with_readme();
        stage_receive_pack_update(
            &mut repo,
            receive_pack_update(vec![("/README.md", Some("will not apply"))]),
        )
        .unwrap();

        let mut catalog = lock_catalog(&state).unwrap();
        catalog.repositories.insert(TEST_REPO_ID.to_string(), repo);
    }
    state.object_store = Arc::new(ReadDeleteFailsObjectStore);

    let response = router(state.clone())
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/repos/owner/repo/staged-update/apply")
                .header(AUTHORIZATION, bearer_header())
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::NOT_FOUND);
    let repo = find_repo(&state, TEST_REPO_OWNER, TEST_REPO_NAME).unwrap();
    assert!(repo.staged_update.is_some());
    assert_eq!(
        repo.live_tree()
            .get(&ScopePath::parse("/README.md").unwrap())
            .map(blob_content)
            .as_deref(),
        Some("hello")
    );
}

#[tokio::test]
async fn reject_staged_update_blob_read_failure_still_discards_update() {
    let mut state = test_state_with_repo();
    cache_test_jwks(&state);
    {
        let mut repo = repo_with_readme();
        stage_receive_pack_update(
            &mut repo,
            receive_pack_update(vec![("/README.md", Some("discard me"))]),
        )
        .unwrap();

        let mut catalog = lock_catalog(&state).unwrap();
        catalog.repositories.insert(TEST_REPO_ID.to_string(), repo);
    }
    state.object_store = Arc::new(ReadDeleteFailsObjectStore);

    let response = router(state.clone())
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/repos/owner/repo/staged-update/reject")
                .header(AUTHORIZATION, bearer_header())
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let body = response_json(response).await;
    assert_eq!(body["line_diff"]["additions"], 0);
    assert_eq!(body["line_diff"]["deletions"], 0);
    let repo = find_repo(&state, TEST_REPO_OWNER, TEST_REPO_NAME).unwrap();
    assert!(repo.staged_update.is_none());
}

struct ReadDeleteFailsObjectStore;

impl crate::object_store::ObjectStore for ReadDeleteFailsObjectStore {
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
