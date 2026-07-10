use super::*;
use crate::domain::requests::{
    RequestActorRole, RequestBaseAudience, StartRequestInput, SubmitRequestInput,
};

pub(super) async fn create_public_request(
    state: &AppState,
    request_id: &str,
    base_main_oid: &str,
    head_oid: &str,
    title: &str,
    stake_ledger_entry_id: &str,
    event_id: &str,
) {
    state
        .metadata
        .start_request(StartRequestInput {
            id: request_id.to_string(),
            repo_id: TEST_REPO_ID.to_string(),
            author_user_id: public_user_id(),
            title: title.to_string(),
            author_role: RequestActorRole::Public,
            base_audience: RequestBaseAudience::Public,
            target_branch: DEFAULT_GIT_BRANCH.to_string(),
            request_ref: canonical_request_ref(request_id),
            base_main_oid: base_main_oid.to_string(),
            now_unix: 2,
        })
        .await
        .unwrap();
    state
        .metadata
        .record_working_request_upload(RecordWorkingRequestUploadInput {
            request_id: request_id.to_string(),
            actor_user_id: public_user_id(),
            actor_can_edit: true,
            expected_old_head_oid: None,
            new_head_oid: head_oid.to_string(),
            git_snapshot: source_blob("public request git snapshot"),
            now_unix: 3,
        })
        .await
        .unwrap();
    state
        .metadata
        .submit_request(SubmitRequestInput {
            request_id: request_id.to_string(),
            actor_user_id: public_user_id(),
            expected_head_oid: head_oid.to_string(),
            stake_credits: 10,
            stake_ledger_entry_id: Some(stake_ledger_entry_id.to_string()),
            event_id: event_id.to_string(),
            now_unix: 4,
        })
        .await
        .unwrap();
}

pub(super) async fn create_owner_request(state: &AppState, request_id: &str, head_oid: &str) {
    state
        .metadata
        .start_request(StartRequestInput {
            id: request_id.to_string(),
            repo_id: TEST_REPO_ID.to_string(),
            author_user_id: test_owner_id(),
            title: "Owner request".to_string(),
            author_role: RequestActorRole::Owner,
            base_audience: RequestBaseAudience::Private,
            target_branch: DEFAULT_GIT_BRANCH.to_string(),
            request_ref: canonical_request_ref(request_id),
            base_main_oid: "base_main".to_string(),
            now_unix: 2,
        })
        .await
        .unwrap();
    state
        .metadata
        .record_working_request_upload(RecordWorkingRequestUploadInput {
            request_id: request_id.to_string(),
            actor_user_id: test_owner_id(),
            actor_can_edit: true,
            expected_old_head_oid: None,
            new_head_oid: head_oid.to_string(),
            git_snapshot: source_blob("owner request git snapshot"),
            now_unix: 3,
        })
        .await
        .unwrap();
    state
        .metadata
        .submit_request(SubmitRequestInput {
            request_id: request_id.to_string(),
            actor_user_id: test_owner_id(),
            expected_head_oid: head_oid.to_string(),
            stake_credits: 0,
            stake_ledger_entry_id: None,
            event_id: format!("event_created_{request_id}"),
            now_unix: 4,
        })
        .await
        .unwrap();
}

pub(super) async fn start_request_via_http(app: axum::Router, bearer: &str) -> serde_json::Value {
    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/repos/owner/repo/requests")
                .header(AUTHORIZATION, bearer)
                .header(CONTENT_TYPE, "application/json")
                .body(Body::from(r#"{"title":"Fix parser crash"}"#))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    response_json(response).await
}

pub(super) async fn submit_request_via_http(
    app: axum::Router,
    bearer: &str,
    request_id: &str,
    body: &str,
) -> Response {
    app.oneshot(
        Request::builder()
            .method("POST")
            .uri(format!("/v1/repos/owner/repo/requests/{request_id}/submit"))
            .header(AUTHORIZATION, bearer)
            .header(CONTENT_TYPE, "application/json")
            .body(Body::from(body.to_string()))
            .unwrap(),
    )
    .await
    .unwrap()
}

pub(super) async fn mark_working_request_uploaded(
    state: &AppState,
    request_id: &str,
    head_oid: &str,
) {
    state
        .metadata
        .record_working_request_upload(RecordWorkingRequestUploadInput {
            request_id: request_id.to_string(),
            actor_user_id: request_author_id(state, request_id).await,
            actor_can_edit: true,
            expected_old_head_oid: None,
            new_head_oid: head_oid.to_string(),
            git_snapshot: source_blob("working request git snapshot"),
            now_unix: 2,
        })
        .await
        .unwrap();
}

async fn request_author_id(state: &AppState, request_id: &str) -> String {
    state
        .metadata
        .request_by_id(request_id)
        .await
        .unwrap()
        .unwrap()
        .author_user_id
}
