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

fn enqueue_repo_cleanup(state: &AppState) {
    lock_catalog(state)
        .unwrap()
        .pending_repo_storage_deletions
        .push(RepoStorageCleanup {
            owner_handle: TEST_REPO_OWNER.to_string(),
            repo_name: TEST_REPO_NAME.to_string(),
        });
}

async fn request(
    state: AppState,
    method: &str,
    uri: &str,
    authorization: String,
    body: Option<String>,
) -> Response {
    let mut request = Request::builder()
        .method(method)
        .uri(uri)
        .header(AUTHORIZATION, authorization);
    let body = if let Some(body) = body {
        request = request.header(CONTENT_TYPE, "application/json");
        Body::from(body)
    } else {
        Body::empty()
    };
    router(state)
        .oneshot(request.body(body).unwrap())
        .await
        .unwrap()
}

#[tokio::test]
async fn create_repo_route_cleans_pending_filesystem_cleanup_before_recreate() {
    let state = test_state_with_jwks();
    let (owner_repo, staged_repo, rx_repo) = cleanup_paths(&state, "recreate");
    enqueue_repo_cleanup(&state);
    let response = request(
        state.clone(),
        "POST",
        "/v1/repos",
        bearer_header(),
        Some(r#"{"name":"Repo"}"#.to_string()),
    )
    .await;

    assert_eq!(response.status(), StatusCode::OK);
    assert!(
        find_repo(&state, TEST_REPO_OWNER, TEST_REPO_NAME)
            .await
            .is_ok()
    );
    assert!(!owner_repo.exists());
    assert!(!staged_repo.exists());
    assert!(!rx_repo.exists());
    assert!(
        lock_catalog(&state)
            .unwrap()
            .pending_repo_storage_deletions
            .is_empty()
    );
}

#[tokio::test]
async fn duplicate_create_does_not_run_pending_filesystem_cleanup_for_live_repo() {
    let state = test_state_with_repo();
    cache_test_jwks(&state);
    let (owner_repo, staged_repo, rx_repo) = cleanup_paths(&state, "live");
    enqueue_repo_cleanup(&state);
    let response = request(
        state.clone(),
        "POST",
        "/v1/repos",
        bearer_header(),
        Some(r#"{"name":"Repo"}"#.to_string()),
    )
    .await;

    assert_eq!(response.status(), StatusCode::CONFLICT);
    assert!(owner_repo.exists());
    assert!(staged_repo.exists());
    assert!(rx_repo.exists());
    assert_eq!(
        lock_catalog(&state)
            .unwrap()
            .pending_repo_storage_deletions
            .len(),
        1
    );
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
    let source_keys = find_repo(&state, TEST_REPO_OWNER, TEST_REPO_NAME)
        .await
        .unwrap()
        .source_blobs()
        .into_iter()
        .map(|blob| blob.object_key)
        .collect::<Vec<_>>();
    let (owner_repo, staged_repo, rx_repo) = cleanup_paths(&state, "test");
    let non_owner = request(
        state.clone(),
        "DELETE",
        "/v1/repos/owner/repo",
        bearer_header_for("user_stranger", "stranger@example.com"),
        None,
    )
    .await;
    assert_eq!(non_owner.status(), StatusCode::NOT_FOUND);

    let response = request(
        state.clone(),
        "DELETE",
        "/v1/repos/owner/repo",
        bearer_header(),
        None,
    )
    .await;

    assert_eq!(response.status(), StatusCode::OK);
    let body = response_json(response).await;
    assert_eq!(body["id"], TEST_REPO_ID);
    assert_eq!(body["deleted"], true);
    assert!(
        find_repo(&state, TEST_REPO_OWNER, TEST_REPO_NAME)
            .await
            .is_err()
    );
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
    let (owner_repo, staged_repo, rx_repo) = cleanup_paths(&state, "delete-fails");
    let response = request(
        state.clone(),
        "DELETE",
        "/v1/repos/owner/repo",
        bearer_header(),
        None,
    )
    .await;

    assert_eq!(response.status(), StatusCode::OK);
    assert!(
        find_repo(&state, TEST_REPO_OWNER, TEST_REPO_NAME)
            .await
            .is_err()
    );
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

#[tokio::test]
async fn delete_repo_route_records_pending_filesystem_cleanup_when_storage_delete_fails() {
    let state = test_state_with_repo();
    cache_test_jwks(&state);
    {
        let mut catalog = lock_catalog(&state).unwrap();
        catalog
            .repositories
            .insert(TEST_REPO_ID.to_string(), repo_with_readme());
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

    let response = request(
        state.clone(),
        "DELETE",
        "/v1/repos/owner/repo",
        bearer_header(),
        None,
    )
    .await;

    assert_eq!(response.status(), StatusCode::OK);
    assert!(
        find_repo(&state, TEST_REPO_OWNER, TEST_REPO_NAME)
            .await
            .is_err()
    );
    assert_eq!(
        lock_catalog(&state)
            .unwrap()
            .pending_repo_storage_deletions
            .len(),
        1
    );
    assert!(!owner_repo.exists());
    assert!(!staged_repo.exists());
    assert!(rx_root.exists());

    fs::remove_file(&rx_root).unwrap();
    drain_pending_repo_storage_deletions(&state).await.unwrap();
    assert!(
        lock_catalog(&state)
            .unwrap()
            .pending_repo_storage_deletions
            .is_empty()
    );
}

#[tokio::test]
async fn pending_repo_storage_cleanup_does_not_delete_recreated_repo_storage() {
    let state = test_state_with_repo();
    let (owner_repo, staged_repo, rx_repo) = cleanup_paths(&state, "recreated");
    {
        let mut catalog = lock_catalog(&state).unwrap();
        catalog
            .repositories
            .insert(TEST_REPO_ID.to_string(), repo_with_readme());
    }
    enqueue_repo_cleanup(&state);

    drain_pending_repo_storage_deletions(&state).await.unwrap();

    assert!(owner_repo.exists());
    assert!(staged_repo.exists());
    assert!(rx_repo.exists());
    assert_eq!(
        lock_catalog(&state)
            .unwrap()
            .pending_repo_storage_deletions
            .len(),
        1
    );

    {
        let mut catalog = lock_catalog(&state).unwrap();
        catalog.repositories.remove(TEST_REPO_ID);
    }
    drain_pending_repo_storage_deletions(&state).await.unwrap();
    assert!(!owner_repo.exists());
    assert!(!staged_repo.exists());
    assert!(!rx_repo.exists());
    assert!(
        lock_catalog(&state)
            .unwrap()
            .pending_repo_storage_deletions
            .is_empty()
    );
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
