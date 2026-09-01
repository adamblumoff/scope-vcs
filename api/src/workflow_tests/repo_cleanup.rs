use super::*;

fn cleanup_paths(state: &AppState, suffix: &str) -> (PathBuf, PathBuf, PathBuf) {
    let incarnation = test_repo_incarnation();
    let raw = state.repository_engine.repository_path(&incarnation);
    let request_refs = request_ref_store_repo_path(state, &incarnation);
    let rx = git_repo_storage_root(state).join("git-rx").join(format!(
        "{}-{suffix}.git",
        receive_pack_staging_repo_prefix(&incarnation)
    ));
    for path in [&raw, &request_refs, &rx] {
        fs::create_dir_all(path).unwrap();
    }
    (raw, request_refs, rx)
}

fn assert_cleanup_paths(paths: &(PathBuf, PathBuf, PathBuf), exist: bool) {
    for path in [&paths.0, &paths.1, &paths.2] {
        assert_eq!(
            path.exists(),
            exist,
            "unexpected state for {}",
            path.display()
        );
    }
}

async fn pending_cleanup_count(state: &AppState) -> usize {
    state
        .metadata
        .cleanup()
        .pending_repo_storage_cleanups_for_tests()
        .await
        .unwrap()
        .len()
}

async fn delete_repo(state: &AppState) -> Response {
    request(
        state.clone(),
        "DELETE",
        "/v1/repos/owner/repo",
        bearer_header(),
    )
    .await
}

async fn assert_repo_deleted(state: &AppState) {
    assert!(
        find_repo(state, TEST_REPO_OWNER, TEST_REPO_NAME)
            .await
            .is_err()
    );
}

async fn request(state: AppState, method: &str, uri: &str, authorization: String) -> Response {
    let request = Request::builder()
        .method(method)
        .uri(uri)
        .header(AUTHORIZATION, authorization);
    router(state)
        .oneshot(request.body(Body::empty()).unwrap())
        .await
        .unwrap()
}

#[tokio::test]
async fn delete_repo_route_requires_owner_and_removes_storage() {
    let state = test_state_with_repo();
    cache_test_jwks(&state);
    let paths = cleanup_paths(&state, "test");
    let non_owner = request(
        state.clone(),
        "DELETE",
        "/v1/repos/owner/repo",
        bearer_header_for("user_stranger", "stranger@example.com"),
    )
    .await;
    assert_eq!(non_owner.status(), StatusCode::NOT_FOUND);

    let response = delete_repo(&state).await;

    assert_eq!(response.status(), StatusCode::OK);
    let body = response_json(response).await;
    assert_eq!(body["id"], TEST_REPO_ID);
    assert_eq!(body["deleted"], true);
    assert_repo_deleted(&state).await;
    assert_cleanup_paths(&paths, false);
}

#[tokio::test]
async fn cleanup_removes_only_the_deleted_repository_incarnation() {
    let state = test_state_with_repo();
    let deleted =
        scope_domain::repository::RepositoryIncarnation::new(TEST_REPO_ID, "repoi_deleted_cleanup")
            .unwrap();
    let recreated = scope_domain::repository::RepositoryIncarnation::new(
        TEST_REPO_ID,
        "repoi_recreated_cleanup",
    )
    .unwrap();
    let deleted_raw = state.repository_engine.repository_path(&deleted);
    let recreated_raw = state.repository_engine.repository_path(&recreated);
    let deleted_refs = request_ref_store_repo_path(&state, &deleted);
    let recreated_refs = request_ref_store_repo_path(&state, &recreated);
    let rx_root = git_repo_storage_root(&state).join("git-rx");
    let deleted_rx = rx_root.join(format!(
        "{}-test.git",
        receive_pack_staging_repo_prefix(&deleted)
    ));
    let recreated_rx = rx_root.join(format!(
        "{}-test.git",
        receive_pack_staging_repo_prefix(&recreated)
    ));
    for path in [
        &deleted_raw,
        &recreated_raw,
        &deleted_refs,
        &recreated_refs,
        &deleted_rx,
        &recreated_rx,
    ] {
        fs::create_dir_all(path).unwrap();
    }

    crate::git::storage::delete_repo_storage(
        &state,
        &scope_domain::repo_actions::RepoStorageCleanup {
            owner_handle: TEST_REPO_OWNER.to_string(),
            repo_name: TEST_REPO_NAME.to_string(),
            incarnation: deleted,
        },
    )
    .unwrap();

    assert!(!deleted_raw.exists());
    assert!(!deleted_refs.exists());
    assert!(!deleted_rx.exists());
    assert!(recreated_raw.exists());
    assert!(recreated_refs.exists());
    assert!(recreated_rx.exists());
}

#[tokio::test]
async fn delete_repo_route_records_pending_cleanup_when_bucket_delete_fails() {
    let mut state = test_state_with_repo();
    cache_test_jwks(&state);
    {
        state
            .metadata
            .repositories()
            .replace_repository_for_tests(repo_with_readme(&state))
            .await
            .unwrap();
    }
    state.object_store = Arc::new(DeleteFailsObjectStore);
    let paths = cleanup_paths(&state, "delete-fails");
    let response = delete_repo(&state).await;

    assert_eq!(response.status(), StatusCode::OK);
    assert_repo_deleted(&state).await;
    assert!(
        !state
            .metadata
            .cleanup()
            .pending_source_blob_cleanups_for_tests()
            .await
            .unwrap()
            .is_empty()
    );
    assert_cleanup_paths(&paths, false);
}

#[tokio::test]
async fn delete_repo_route_records_pending_filesystem_cleanup_when_storage_delete_fails() {
    let state = test_state_with_repo();
    cache_test_jwks(&state);
    {
        state
            .metadata
            .repositories()
            .replace_repository_for_tests(repo_with_readme(&state))
            .await
            .unwrap();
    }
    let raw_repo = state
        .repository_engine
        .repository_path(&test_repo_incarnation());
    let request_ref_repo = request_ref_store_repo_path(&state, &test_repo_incarnation());
    let storage_root = git_repo_storage_root(&state);
    let rx_root = storage_root.join("git-rx");
    fs::create_dir_all(&raw_repo).unwrap();
    fs::create_dir_all(&request_ref_repo).unwrap();
    fs::create_dir_all(&storage_root).unwrap();
    if rx_root.is_dir() {
        fs::remove_dir_all(&rx_root).unwrap();
    } else if rx_root.exists() {
        fs::remove_file(&rx_root).unwrap();
    }
    fs::write(&rx_root, "not a directory").unwrap();

    let response = delete_repo(&state).await;

    assert_eq!(response.status(), StatusCode::OK);
    assert_repo_deleted(&state).await;
    assert_eq!(pending_cleanup_count(&state).await, 1);
    assert!(!raw_repo.exists());
    assert!(!request_ref_repo.exists());
    assert!(rx_root.exists());

    fs::remove_file(&rx_root).unwrap();
    drain_pending_repo_storage_deletions(&state).await.unwrap();
    assert_eq!(pending_cleanup_count(&state).await, 0);
}

struct DeleteFailsObjectStore;

impl scope_object_store::ObjectStore for DeleteFailsObjectStore {
    fn put(&self, _key: &str, _bytes: &[u8]) -> Result<(), scope_object_store::ObjectStoreError> {
        Ok(())
    }

    fn get(&self, key: &str) -> Result<Vec<u8>, scope_object_store::ObjectStoreError> {
        Err(scope_object_store::ObjectStoreError::not_found(format!(
            "object {key} not found"
        )))
    }

    fn delete(&self, _key: &str) -> Result<(), scope_object_store::ObjectStoreError> {
        Err(scope_object_store::ObjectStoreError::service_unavailable(
            "delete failed",
        ))
    }
}
