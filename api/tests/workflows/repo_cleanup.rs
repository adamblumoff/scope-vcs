use super::*;

fn cleanup_paths(state: &AppState, suffix: &str) -> (PathBuf, PathBuf, PathBuf) {
    let owner = owner_git_repo_path(state, TEST_REPO_OWNER, TEST_REPO_NAME);
    let staged = staged_git_repo_path(state, TEST_REPO_OWNER, TEST_REPO_NAME);
    let rx = git_repo_storage_root(state).join("git-rx").join(format!(
        "{}-{suffix}.git",
        receive_pack_staging_repo_prefix(TEST_REPO_OWNER, TEST_REPO_NAME)
    ));
    for path in [&owner, &staged, &rx] {
        fs::create_dir_all(path).unwrap();
    }
    (owner, staged, rx)
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
async fn delete_repo_route_records_pending_cleanup_when_bucket_delete_fails() {
    let mut state = test_state_with_repo();
    cache_test_jwks(&state);
    {
        state
            .metadata
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
            .replace_repository_for_tests(repo_with_readme(&state))
            .await
            .unwrap();
    }
    let owner_repo = owner_git_repo_path(&state, TEST_REPO_OWNER, TEST_REPO_NAME);
    let staged_repo = staged_git_repo_path(&state, TEST_REPO_OWNER, TEST_REPO_NAME);
    let storage_root = git_repo_storage_root(&state);
    let rx_root = storage_root.join("git-rx");
    fs::create_dir_all(&owner_repo).unwrap();
    fs::create_dir_all(&staged_repo).unwrap();
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
    assert!(!owner_repo.exists());
    assert!(!staged_repo.exists());
    assert!(rx_root.exists());

    fs::remove_file(&rx_root).unwrap();
    drain_pending_repo_storage_deletions(&state).await.unwrap();
    assert_eq!(pending_cleanup_count(&state).await, 0);
}

struct DeleteFailsObjectStore;

impl crate::object_store::ObjectStore for DeleteFailsObjectStore {
    fn put(&self, _key: &str, _bytes: &[u8]) -> Result<(), scope_core::error::ApiError> {
        Ok(())
    }

    fn get(&self, key: &str) -> Result<Vec<u8>, scope_core::error::ApiError> {
        Err(scope_core::error::ApiError::not_found(format!(
            "object {key} not found"
        )))
    }

    fn delete(&self, _key: &str) -> Result<(), scope_core::error::ApiError> {
        Err(scope_core::error::ApiError::service_unavailable(
            "delete failed",
        ))
    }
}
