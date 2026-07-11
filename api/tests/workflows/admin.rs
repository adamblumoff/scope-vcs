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
    let blob = put_source_blob(state.object_store.as_ref(), TEST_REPO_ID, bytes).unwrap();
    let key = blob.object_key.clone();
    state
        .metadata
        .queue_pending_source_blob_deletions(vec![blob])
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

async fn reset(state: AppState, confirm: &str) -> Response {
    admin_request(
        state,
        "POST",
        "/v1/admin/metadata/reset",
        Some(OPERATOR_AUTH.into()),
        Body::from(format!(
            r#"{{"confirm":"{confirm}","reason":"reset from test"}}"#
        )),
    )
    .await
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
async fn admin_cleanup_status_shows_pending_cleanup_and_reset_events() {
    let state = operator_state();
    let reset_event = state.metadata.reset_catalog("test reset").await.unwrap();
    queued_blob(&state, b"pending").await;
    state
        .metadata
        .queue_repo_storage_cleanup_for_tests(RepoStorageCleanup {
            owner_handle: TEST_REPO_OWNER.to_string(),
            repo_name: TEST_REPO_NAME.to_string(),
        })
        .await
        .unwrap();
    let response = cleanup_status(state, Some(OPERATOR_AUTH.into())).await;
    assert_eq!(response.status(), StatusCode::OK);
    let body = response_json(response).await;
    assert_eq!(body["pending_cleanup"]["repo_storage"]["count"], 1);
    assert_eq!(body["pending_cleanup"]["source_blob_deletes"]["count"], 1);
    assert_eq!(body["metadata_resets"]["count"], 1);
    assert_eq!(body["metadata_resets"]["events"][0]["id"], reset_event.id);
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
            .pending_source_blob_cleanups_for_tests()
            .await
            .unwrap()
            .is_empty()
    );

    let mut state = operator_state();
    state.object_store = Arc::new(DeleteFailsObjectStore);
    let blob = crate::object_store::repo_object_for_bytes("blobs", "admin-delete-fails", b"stale");
    let key = blob.object_key.clone();
    state
        .metadata
        .queue_pending_source_blob_deletions(vec![blob])
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
            .pending_source_blob_cleanups_for_tests()
            .await
            .unwrap()
            .iter()
            .any(|blob| blob.object_key == key)
    );
}

#[tokio::test]
async fn admin_metadata_reset_requires_confirmation_and_clears_catalog() {
    let state = operator_state();

    let rejected = reset(state.clone(), "wrong").await;
    assert_eq!(rejected.status(), StatusCode::BAD_REQUEST);
    assert!(
        state
            .metadata
            .repository_for_tests(TEST_REPO_ID)
            .await
            .unwrap()
            .is_some()
    );

    let response = reset(state.clone(), "reset-pre-alpha-metadata").await;
    assert_eq!(response.status(), StatusCode::OK);
    let body = response_json(response).await;
    assert_eq!(body["event"]["trigger"], "operator");
    assert_eq!(body["event"]["reason"], "reset from test");
    assert_eq!(
        state.metadata.repository_count_for_tests().await.unwrap(),
        0
    );
    assert!(
        state
            .metadata
            .pending_source_blob_cleanups_for_tests()
            .await
            .unwrap()
            .is_empty()
    );
}

fn operator_state() -> AppState {
    let mut state = test_state_with_repo();
    state.operator_token = Some(Arc::<str>::from(OPERATOR_TOKEN));
    state
}

struct DeleteFailsObjectStore;

impl crate::object_store::ObjectStore for DeleteFailsObjectStore {
    fn put(&self, _key: &str, _bytes: &[u8]) -> Result<(), scope_core::error::ApiError> {
        Ok(())
    }

    fn get(&self, _key: &str) -> Result<Vec<u8>, scope_core::error::ApiError> {
        Err(scope_core::error::ApiError::not_found("object not found"))
    }

    fn delete(&self, _key: &str) -> Result<(), scope_core::error::ApiError> {
        Err(scope_core::error::ApiError::service_unavailable(
            "delete failed",
        ))
    }
}
