use super::*;
use crate::domain::requests::{
    RecordWorkingRequestUploadInput, RequestActorRole, RequestAudience, StartRequestInput,
};

pub(crate) async fn create_owner_request(state: &AppState, request_id: &str, head_oid: &str) {
    create_request(RequestFixture {
        state,
        request_id,
        author_user_id: test_owner_id(),
        title: "Owner request",
        role: RequestActorRole::Owner,
        audience: RequestAudience::Private,
        base_main_oid: "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        head_oid,
        snapshot: "owner request git snapshot",
    })
    .await;
}

struct RequestFixture<'a> {
    state: &'a AppState,
    request_id: &'a str,
    author_user_id: String,
    title: &'a str,
    role: RequestActorRole,
    audience: RequestAudience,
    base_main_oid: &'a str,
    head_oid: &'a str,
    snapshot: &'a str,
}

async fn create_request(fixture: RequestFixture<'_>) {
    fixture
        .state
        .metadata
        .start_request(StartRequestInput {
            id: fixture.request_id.to_string(),
            repo_id: TEST_REPO_ID.to_string(),
            name: request_name(fixture.request_id),
            author_user_id: fixture.author_user_id.clone(),
            title: Some(fixture.title.to_string()),
            author_role: fixture.role,
            audience: fixture.audience,
            base_main_oid: fixture.base_main_oid.to_string(),
            event_id: format!("event_{}_started", fixture.request_id),
            now_unix: 2,
        })
        .await
        .unwrap();
    let mut git_snapshot = source_blob(fixture.state, fixture.snapshot);
    git_snapshot.git_oid = fixture.head_oid.to_string();
    fixture
        .state
        .metadata
        .record_working_request_upload(RecordWorkingRequestUploadInput {
            request_id: fixture.request_id.to_string(),
            actor_user_id: fixture.author_user_id.clone(),
            actor_can_edit: true,
            expected_old_head_oid: None,
            new_head_oid: fixture.head_oid.to_string(),
            git_snapshot,
            now_unix: 3,
        })
        .await
        .unwrap();
}

fn request_name(request_id: &str) -> String {
    request_id.replace('_', "-")
}
