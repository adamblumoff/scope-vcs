use super::*;

const OPERATOR_TOKEN: &str = "operator-secret";

#[tokio::test]
async fn admin_cleanup_requires_configured_operator_token() {
    let app = router(AppState::test_state());
    let unconfigured = app
        .oneshot(
            Request::builder()
                .uri("/v1/admin/cleanup")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(unconfigured.status(), StatusCode::SERVICE_UNAVAILABLE);

    let app = router(operator_state());
    let missing = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/v1/admin/cleanup")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(missing.status(), StatusCode::UNAUTHORIZED);

    let wrong = app
        .oneshot(
            Request::builder()
                .uri("/v1/admin/cleanup")
                .header(AUTHORIZATION, "Bearer wrong-token")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(wrong.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn admin_cleanup_status_shows_pending_cleanup_and_reset_events() {
    let state = operator_state();
    let reset_event = state.metadata.reset_catalog("test reset").unwrap();
    let pending_blob = put_source_blob(state.object_store.as_ref(), TEST_REPO_ID, b"pending")
        .expect("pending blob is stored");
    let pending_blob_key = pending_blob.object_key.clone();
    state
        .metadata
        .update(move |catalog| {
            catalog
                .pending_repo_storage_deletions
                .push(RepoStorageCleanup {
                    owner_handle: TEST_REPO_OWNER.to_string(),
                    repo_name: TEST_REPO_NAME.to_string(),
                });
            catalog.pending_source_blob_deletions.push(pending_blob);
            Ok(())
        })
        .unwrap();

    let response = router(state)
        .oneshot(
            Request::builder()
                .uri("/v1/admin/cleanup")
                .header(AUTHORIZATION, operator_auth())
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = response_json(response).await;
    assert_eq!(body["pending_cleanup"]["repo_storage"]["count"], 1);
    assert_eq!(body["pending_cleanup"]["source_blob_deletes"]["count"], 1);
    assert_eq!(
        body["failed_object_deletes"]["objects"][0]["object_key"],
        pending_blob_key
    );
    assert_eq!(body["metadata_resets"]["count"], 1);
    assert_eq!(body["metadata_resets"]["events"][0]["id"], reset_event.id);
}

#[tokio::test]
async fn admin_cleanup_drain_deletes_pending_source_blobs() {
    let state = operator_state();
    let pending_blob = put_source_blob(state.object_store.as_ref(), TEST_REPO_ID, b"stale")
        .expect("pending blob is stored");
    let pending_blob_key = pending_blob.object_key.clone();
    state
        .metadata
        .update(move |catalog| {
            catalog.pending_source_blob_deletions.push(pending_blob);
            Ok(())
        })
        .unwrap();

    let response = router(state.clone())
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/admin/cleanup/drain")
                .header(AUTHORIZATION, operator_auth())
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = response_json(response).await;
    assert_eq!(body["status"], "drained");
    assert_eq!(body["report"]["source_blobs"]["attempted"], 1);
    assert_eq!(body["report"]["source_blobs"]["deleted"], 1);
    assert_eq!(body["report"]["source_blobs"]["retained"], 0);
    assert!(state.object_store.get(&pending_blob_key).is_err());
    assert!(
        state
            .metadata
            .read(|catalog| Ok(catalog.pending_source_blob_deletions.is_empty()))
            .unwrap()
    );
}

#[tokio::test]
async fn admin_cleanup_drain_reports_failed_object_deletes() {
    let mut state = operator_state();
    state.object_store = Arc::new(DeleteFailsObjectStore);
    let pending_blob =
        crate::object_store::repo_object_for_bytes("blobs", "admin-delete-fails", b"stale");
    let pending_blob_key = pending_blob.object_key.clone();
    state
        .metadata
        .update(move |catalog| {
            catalog.pending_source_blob_deletions.push(pending_blob);
            Ok(())
        })
        .unwrap();

    let response = router(state.clone())
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/admin/cleanup/drain")
                .header(AUTHORIZATION, operator_auth())
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    let body = response_json(response).await;
    assert_eq!(body["status"], "failed");
    assert_eq!(
        body["report"]["source_blobs"]["failed_object_deletes"][0]["object_key"],
        pending_blob_key
    );
    assert_eq!(body["report"]["source_blobs"]["retained"], 1);
    assert!(
        state
            .metadata
            .read(|catalog| Ok(catalog
                .pending_source_blob_deletions
                .iter()
                .any(|blob| blob.object_key == pending_blob_key)))
            .unwrap()
    );
}

#[tokio::test]
async fn admin_metadata_reset_requires_confirmation_and_clears_catalog() {
    let state = operator_state();

    let rejected = router(state.clone())
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/admin/metadata/reset")
                .header(AUTHORIZATION, operator_auth())
                .header(CONTENT_TYPE, "application/json")
                .body(Body::from(
                    r#"{"confirm":"wrong","reason":"reset from test"}"#,
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(rejected.status(), StatusCode::BAD_REQUEST);
    assert!(
        state
            .metadata
            .read(|catalog| Ok(catalog.repositories.contains_key(TEST_REPO_ID)))
            .unwrap()
    );

    let response = router(state.clone())
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/admin/metadata/reset")
                .header(AUTHORIZATION, operator_auth())
                .header(CONTENT_TYPE, "application/json")
                .body(Body::from(
                    r#"{"confirm":"reset-pre-alpha-metadata","reason":"reset from test"}"#,
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = response_json(response).await;
    assert_eq!(body["event"]["trigger"], "operator");
    assert_eq!(body["event"]["reason"], "reset from test");
    assert!(
        state
            .metadata
            .read(|catalog| Ok(catalog.repositories.is_empty()
                && catalog.pending_repo_storage_deletions.is_empty()
                && catalog.pending_source_blob_deletions.is_empty()))
            .unwrap()
    );
    assert_eq!(state.metadata.metadata_reset_events().unwrap().len(), 1);
}

fn operator_state() -> AppState {
    let mut state = test_state_with_repo();
    state.operator_token = Some(Arc::<str>::from(OPERATOR_TOKEN));
    state
}

fn operator_auth() -> String {
    format!("Bearer {OPERATOR_TOKEN}")
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
