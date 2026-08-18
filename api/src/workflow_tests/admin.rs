use super::*;

const OPERATOR_TOKEN: &str = "operator-secret";
const OPERATOR_AUTH: &str = "Bearer operator-secret";

async fn admin_request(
    state: AppState,
    method: &str,
    uri: &str,
    auth: Option<String>,
    body: Body,
) -> Response {
    let mut request = Request::builder().method(method).uri(uri);
    if let Some(auth) = auth {
        request = request.header(AUTHORIZATION, auth);
    }
    request = request.header(CONTENT_TYPE, "application/json");
    router(state)
        .oneshot(request.body(body).unwrap())
        .await
        .unwrap()
}

async fn queued_blob(state: &AppState, bytes: &[u8]) -> String {
    let blob = put_source_blob(state.object_store.as_ref(), bytes).unwrap();
    let key = scope_object_store::object_key(&blob);
    state
        .metadata
        .cleanup()
        .queue_pending_source_blob_deletions(
            vec![blob],
            unix_now(),
            &crate::persistence_ids::generate_persistence_id,
        )
        .await
        .unwrap();
    key
}

async fn drain(state: AppState) -> Response {
    admin_request(
        state,
        "POST",
        "/v1/admin/cleanup/drain",
        Some(OPERATOR_AUTH.into()),
        Body::empty(),
    )
    .await
}

async fn cleanup_status(state: AppState, auth: Option<String>) -> Response {
    admin_request(state, "GET", "/v1/admin/cleanup", auth, Body::empty()).await
}

#[tokio::test]
async fn admin_cleanup_requires_configured_operator_token() {
    for (state, auth, status) in [
        (
            AppState::test_state(),
            None,
            StatusCode::SERVICE_UNAVAILABLE,
        ),
        (operator_state(), None, StatusCode::UNAUTHORIZED),
        (
            operator_state(),
            Some("Bearer wrong-token".into()),
            StatusCode::UNAUTHORIZED,
        ),
    ] {
        assert_eq!(cleanup_status(state, auth).await.status(), status);
    }
}

#[tokio::test]
async fn admin_cleanup_status_shows_pending_cleanup_queues() {
    let state = operator_state();
    queued_blob(&state, b"pending").await;
    state
        .metadata
        .cleanup()
        .queue_repo_storage_cleanup_for_tests(
            RepoStorageCleanup {
                owner_handle: TEST_REPO_OWNER.to_string(),
                repo_name: TEST_REPO_NAME.to_string(),
            },
            unix_now(),
        )
        .await
        .unwrap();
    let response = cleanup_status(state, Some(OPERATOR_AUTH.into())).await;
    assert_eq!(response.status(), StatusCode::OK);
    let body = response_json(response).await;
    assert_eq!(body["pending_cleanup"]["repo_storage"]["count"], 1);
    assert_eq!(body["pending_cleanup"]["source_blob_deletes"]["count"], 1);
    assert!(body.get("metadata_resets").is_none());
}

#[tokio::test]
async fn admin_cleanup_drain_reports_deleted_and_failed_source_blobs() {
    let state = operator_state();
    let key = queued_blob(&state, b"stale").await;
    let response = drain(state.clone()).await;
    assert_eq!(response.status(), StatusCode::OK);
    let body = response_json(response).await;
    assert_eq!(body["status"], "drained");
    assert_eq!(body["report"]["source_blobs"]["attempted"], 1);
    assert_eq!(body["report"]["source_blobs"]["deleted"], 1);
    assert_eq!(body["report"]["source_blobs"]["retained"], 0);
    assert!(state.object_store.get(&key).is_err());
    assert!(
        state
            .metadata
            .cleanup()
            .pending_source_blob_cleanups_for_tests()
            .await
            .unwrap()
            .is_empty()
    );

    let mut state = operator_state();
    state.object_store = Arc::new(DeleteFailsObjectStore);
    let blob = scope_object_store::content_object_for_bytes(
        scope_object_store::ContentObjectKind::Blob,
        b"stale",
    );
    let key = scope_object_store::object_key(&blob);
    state
        .metadata
        .cleanup()
        .queue_pending_source_blob_deletions(
            vec![blob],
            unix_now(),
            &crate::persistence_ids::generate_persistence_id,
        )
        .await
        .unwrap();
    let response = drain(state.clone()).await;
    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    let body = response_json(response).await;
    assert_eq!(body["status"], "failed");
    assert_eq!(
        body["report"]["source_blobs"]["failed_object_deletes"][0]["object_key"],
        key
    );
    assert_eq!(body["report"]["source_blobs"]["retained"], 1);
    assert!(
        state
            .metadata
            .cleanup()
            .pending_source_blob_cleanups_for_tests()
            .await
            .unwrap()
            .iter()
            .any(|blob| scope_object_store::object_key(blob) == key)
    );
}

#[tokio::test]
async fn admin_metadata_reset_route_is_absent() {
    let response = admin_request(
        operator_state(),
        "POST",
        "/v1/admin/metadata/reset",
        Some(OPERATOR_AUTH.into()),
        Body::from(r#"{"confirm":"reset-pre-alpha-metadata"}"#),
    )
    .await;
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}

fn operator_state() -> AppState {
    let mut state = test_state_with_repo();
    state.operator_token = Some(Arc::<str>::from(OPERATOR_TOKEN));
    state
}

struct DeleteFailsObjectStore;

impl scope_object_store::ObjectStore for DeleteFailsObjectStore {
    fn put(&self, _key: &str, _bytes: &[u8]) -> Result<(), scope_object_store::ObjectStoreError> {
        Ok(())
    }

    fn get(&self, _key: &str) -> Result<Vec<u8>, scope_object_store::ObjectStoreError> {
        Err(scope_object_store::ObjectStoreError::not_found(
            "object not found",
        ))
    }

    fn delete(&self, _key: &str) -> Result<(), scope_object_store::ObjectStoreError> {
        Err(scope_object_store::ObjectStoreError::service_unavailable(
            "delete failed",
        ))
    }
}
