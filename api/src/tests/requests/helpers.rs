use super::*;
use crate::domain::requests::{
    FinalizeReservedRequestInput, RequestActorRole, RequestBaseAudience, ReserveRequestInput,
};

pub(super) fn create_public_request(
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
        .reserve_request(ReserveRequestInput {
            id: request_id.to_string(),
            repo_id: TEST_REPO_ID.to_string(),
            author_user_id: public_user_id(),
            author_role: RequestActorRole::Public,
            base_audience: RequestBaseAudience::Public,
            target_branch: DEFAULT_GIT_BRANCH.to_string(),
            request_ref: canonical_request_ref(request_id),
            base_main_oid: base_main_oid.to_string(),
            now_unix: 2,
        })
        .unwrap();
    state
        .metadata
        .record_reserved_request_upload(RecordReservedRequestUploadInput {
            request_id: request_id.to_string(),
            actor_user_id: public_user_id(),
            expected_old_head_oid: None,
            new_head_oid: head_oid.to_string(),
            git_snapshot: source_blob("public request git snapshot"),
            now_unix: 3,
        })
        .unwrap();
    state
        .metadata
        .finalize_reserved_request(FinalizeReservedRequestInput {
            request_id: request_id.to_string(),
            actor_user_id: public_user_id(),
            title: title.to_string(),
            expected_head_oid: head_oid.to_string(),
            stake_credits: 10,
            stake_ledger_entry_id: Some(stake_ledger_entry_id.to_string()),
            event_id: event_id.to_string(),
            now_unix: 4,
        })
        .unwrap();
}

pub(super) fn create_owner_request(state: &AppState, request_id: &str, head_oid: &str) {
    state
        .metadata
        .reserve_request(ReserveRequestInput {
            id: request_id.to_string(),
            repo_id: TEST_REPO_ID.to_string(),
            author_user_id: test_owner_id(),
            author_role: RequestActorRole::Owner,
            base_audience: RequestBaseAudience::Private,
            target_branch: DEFAULT_GIT_BRANCH.to_string(),
            request_ref: canonical_request_ref(request_id),
            base_main_oid: "base_main".to_string(),
            now_unix: 2,
        })
        .unwrap();
    state
        .metadata
        .record_reserved_request_upload(RecordReservedRequestUploadInput {
            request_id: request_id.to_string(),
            actor_user_id: test_owner_id(),
            expected_old_head_oid: None,
            new_head_oid: head_oid.to_string(),
            git_snapshot: source_blob("owner request git snapshot"),
            now_unix: 3,
        })
        .unwrap();
    state
        .metadata
        .finalize_reserved_request(FinalizeReservedRequestInput {
            request_id: request_id.to_string(),
            actor_user_id: test_owner_id(),
            title: "Owner request".to_string(),
            expected_head_oid: head_oid.to_string(),
            stake_credits: 0,
            stake_ledger_entry_id: None,
            event_id: format!("event_created_{request_id}"),
            now_unix: 4,
        })
        .unwrap();
}

pub(super) async fn reserve_request_via_http(app: axum::Router, bearer: &str) -> serde_json::Value {
    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/repos/owner/repo/requests/reservations")
                .header(AUTHORIZATION, bearer)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    response_json(response).await
}

pub(super) async fn finalize_request_via_http(
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

pub(super) fn mark_reserved_request_uploaded(state: &AppState, request_id: &str, head_oid: &str) {
    state
        .metadata
        .record_reserved_request_upload(RecordReservedRequestUploadInput {
            request_id: request_id.to_string(),
            actor_user_id: request_author_id(state, request_id),
            expected_old_head_oid: None,
            new_head_oid: head_oid.to_string(),
            git_snapshot: source_blob("reserved request git snapshot"),
            now_unix: 2,
        })
        .unwrap();
}

fn request_author_id(state: &AppState, request_id: &str) -> String {
    state
        .metadata
        .request_by_id(request_id)
        .unwrap()
        .unwrap()
        .author_user_id
}
