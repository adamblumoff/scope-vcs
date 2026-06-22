use super::*;

#[tokio::test]
async fn create_repo_route_cleans_pending_filesystem_cleanup_before_recreate() {
    let state = test_state_with_jwks();
    let owner_repo = owner_git_repo_path(&state, TEST_REPO_OWNER, TEST_REPO_NAME);
    let staged_repo = staged_git_repo_path(&state, TEST_REPO_OWNER, TEST_REPO_NAME);
    let rx_repo = git_repo_storage_root(&state).join("git-rx").join(format!(
        "{}-recreate.git",
        receive_pack_staging_repo_prefix(TEST_REPO_OWNER, TEST_REPO_NAME)
    ));
    fs::create_dir_all(&owner_repo).unwrap();
    fs::create_dir_all(&staged_repo).unwrap();
    fs::create_dir_all(&rx_repo).unwrap();
    {
        let mut catalog = lock_catalog(&state).unwrap();
        catalog
            .pending_repo_storage_deletions
            .push(RepoStorageCleanup {
                owner_handle: TEST_REPO_OWNER.to_string(),
                repo_name: TEST_REPO_NAME.to_string(),
            });
    }
    let app = router(state.clone());

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/repos")
                .header(AUTHORIZATION, bearer_header())
                .header(CONTENT_TYPE, "application/json")
                .body(Body::from(r#"{"name":"Repo"}"#))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    assert!(find_repo(&state, TEST_REPO_OWNER, TEST_REPO_NAME).is_ok());
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
    let owner_repo = owner_git_repo_path(&state, TEST_REPO_OWNER, TEST_REPO_NAME);
    let staged_repo = staged_git_repo_path(&state, TEST_REPO_OWNER, TEST_REPO_NAME);
    let rx_repo = git_repo_storage_root(&state).join("git-rx").join(format!(
        "{}-live.git",
        receive_pack_staging_repo_prefix(TEST_REPO_OWNER, TEST_REPO_NAME)
    ));
    fs::create_dir_all(&owner_repo).unwrap();
    fs::create_dir_all(&staged_repo).unwrap();
    fs::create_dir_all(&rx_repo).unwrap();
    {
        let mut catalog = lock_catalog(&state).unwrap();
        catalog
            .pending_repo_storage_deletions
            .push(RepoStorageCleanup {
                owner_handle: TEST_REPO_OWNER.to_string(),
                repo_name: TEST_REPO_NAME.to_string(),
            });
    }
    let app = router(state.clone());

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/repos")
                .header(AUTHORIZATION, bearer_header())
                .header(CONTENT_TYPE, "application/json")
                .body(Body::from(r#"{"name":"Repo"}"#))
                .unwrap(),
        )
        .await
        .unwrap();

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
    fs::write(&rx_root, "not a directory").unwrap();

    let response = router(state.clone())
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
    drain_pending_repo_storage_deletions(&state).unwrap();
    assert!(
        lock_catalog(&state)
            .unwrap()
            .pending_repo_storage_deletions
            .is_empty()
    );
}

#[test]
fn pending_repo_storage_cleanup_does_not_delete_recreated_repo_storage() {
    let state = test_state_with_repo();
    let owner_repo = owner_git_repo_path(&state, TEST_REPO_OWNER, TEST_REPO_NAME);
    let staged_repo = staged_git_repo_path(&state, TEST_REPO_OWNER, TEST_REPO_NAME);
    let rx_repo = git_repo_storage_root(&state).join("git-rx").join(format!(
        "{}-recreated.git",
        receive_pack_staging_repo_prefix(TEST_REPO_OWNER, TEST_REPO_NAME)
    ));
    fs::create_dir_all(&owner_repo).unwrap();
    fs::create_dir_all(&staged_repo).unwrap();
    fs::create_dir_all(&rx_repo).unwrap();
    {
        let mut catalog = lock_catalog(&state).unwrap();
        catalog
            .repositories
            .insert(TEST_REPO_ID.to_string(), repo_with_readme());
        catalog
            .pending_repo_storage_deletions
            .push(RepoStorageCleanup {
                owner_handle: TEST_REPO_OWNER.to_string(),
                repo_name: TEST_REPO_NAME.to_string(),
            });
    }

    drain_pending_repo_storage_deletions(&state).unwrap();

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
    drain_pending_repo_storage_deletions(&state).unwrap();
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
async fn delete_repo_route_leaves_storage_when_metadata_persist_fails() {
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
    let rx_repo = git_repo_storage_root(&state).join("git-rx").join(format!(
        "{}-persist-fails.git",
        receive_pack_staging_repo_prefix(TEST_REPO_OWNER, TEST_REPO_NAME)
    ));
    fs::create_dir_all(&owner_repo).unwrap();
    fs::create_dir_all(&staged_repo).unwrap();
    fs::create_dir_all(&rx_repo).unwrap();
    state.metadata.fail_next_persist_for_tests().unwrap();

    let response = router(state.clone())
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

    assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
    assert!(find_repo(&state, TEST_REPO_OWNER, TEST_REPO_NAME).is_ok());
    assert!(
        lock_catalog(&state)
            .unwrap()
            .pending_source_blob_deletions
            .is_empty()
    );
    assert!(owner_repo.exists());
    assert!(staged_repo.exists());
    assert!(rx_repo.exists());
    let _ = fs::remove_dir_all(owner_repo);
    let _ = fs::remove_dir_all(staged_repo);
    let _ = fs::remove_dir_all(rx_repo);
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
