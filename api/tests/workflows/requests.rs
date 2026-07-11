use super::*;
use crate::domain::requests::{
    GrantUserCreditsInput, RecordWorkingRequestUploadInput, canonical_request_ref,
};
use tokio_stream::StreamExt;

mod editors;
mod helpers;
pub(super) use helpers::create_owner_request;
use helpers::{
    create_public_request, mark_working_request_uploaded, public_merge_fixture,
    start_request_via_http, submit_request_via_http,
};

const PUBLIC_SUBJECT: &str = "public_requester";
const PUBLIC_EMAIL: &str = "public@example.com";
const REQUEST_HEAD: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";

#[tokio::test]
async fn public_submit_stakes_credits_and_uses_public_base() {
    let state = state_with_public_user().await;
    grant_public_credits(&state, "ledger_grant").await;

    let app = router(state.clone());
    let start = start_request_via_http(
        app.clone(),
        &bearer_header_for(PUBLIC_SUBJECT, PUBLIC_EMAIL),
    )
    .await;
    assert_eq!(
        start["request"]["request_ref"],
        canonical_request_ref(start["request"]["id"].as_str().unwrap())
    );
    mark_working_request_uploaded(
        &state,
        start["request"]["id"].as_str().unwrap(),
        REQUEST_HEAD,
    )
    .await;

    let response = submit_request_via_http(
        app,
        &bearer_header_for(PUBLIC_SUBJECT, PUBLIC_EMAIL),
        start["request"]["id"].as_str().unwrap(),
        r#"{"head_oid":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","stake_credits":10}"#,
    )
    .await;

    assert_eq!(response.status(), StatusCode::OK);
    let body = response_json(response).await;
    assert_eq!(body["request"]["author_role"], "Public");
    assert_eq!(body["request"]["base_audience"], "Public");
    assert_eq!(body["request"]["stake_credits"], 10);
}

#[tokio::test]
async fn request_submit_publishes_summary_refresh_event() {
    let state = test_state_with_repo_with_readme().await;
    let app = router(state.clone());
    let owner_bearer = bearer_header();
    let events = api_request(
        app.clone(),
        "GET",
        "/v1/repos/owner/repo/events",
        Some(&owner_bearer),
        None,
    )
    .await;
    assert_eq!(events.status(), StatusCode::OK);
    let mut stream = events.into_body().into_data_stream();
    let initial = stream.next().await.unwrap().unwrap();
    assert!(
        String::from_utf8(initial.to_vec())
            .unwrap()
            .contains(r#""reason":"connected""#)
    );

    let start = start_request_via_http(app.clone(), &bearer_header()).await;
    let started_event = tokio::time::timeout(std::time::Duration::from_secs(5), stream.next())
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    let started_event = String::from_utf8(started_event.to_vec()).unwrap();
    assert!(started_event.contains(r#""reason":"request-started""#));

    mark_working_request_uploaded(
        &state,
        start["request"]["id"].as_str().unwrap(),
        REQUEST_HEAD,
    )
    .await;
    let submit = submit_request_via_http(
        app,
        &bearer_header(),
        start["request"]["id"].as_str().unwrap(),
        r#"{"head_oid":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"}"#,
    )
    .await;
    assert_eq!(submit.status(), StatusCode::OK);

    let event = tokio::time::timeout(std::time::Duration::from_secs(5), stream.next())
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    let event = String::from_utf8(event.to_vec()).unwrap();
    assert!(event.contains(r#""reason":"request-submitted""#));
    assert!(event.contains(r#""version":0"#));
}

#[tokio::test]
async fn public_readers_do_not_see_private_request_branches() {
    let state = test_state_with_repo();
    cache_test_jwks(&state);
    create_owner_request(&state, "req_private", REQUEST_HEAD).await;
    let app = router(state);

    let public_response = api_request(
        app.clone(),
        "GET",
        "/v1/repos/owner/repo/requests",
        None,
        None,
    )
    .await;

    assert_eq!(public_response.status(), StatusCode::OK);
    let public_body = response_json(public_response).await;
    assert_eq!(public_body["requests"].as_array().unwrap().len(), 0);

    let owner_response = api_request(
        app,
        "GET",
        "/v1/repos/owner/repo/requests",
        Some(&bearer_header()),
        None,
    )
    .await;

    assert_eq!(owner_response.status(), StatusCode::OK);
    let owner_body = response_json(owner_response).await;
    assert_eq!(owner_body["requests"].as_array().unwrap().len(), 1);
    assert_eq!(owner_body["requests"][0]["base_audience"], "Private");
}

#[tokio::test]
async fn public_request_merge_replays_public_delta_without_deleting_private_files() {
    let fixture = public_merge_fixture(
        "req_public_merge",
        &[
            ("README.md", "hello from public request\n"),
            ("ignored.txt", "tracked despite ignore\n"),
        ],
        true,
    )
    .await;

    let merge_body = format!(
        r#"{{"expected_main_oid":"{}","expected_head_oid":"{}"}}"#,
        fixture.raw_main_oid, fixture.request_head
    );
    let response = api_request(
        router(fixture.state.clone()),
        "POST",
        "/v1/repos/owner/repo/requests/req_public_merge/merge",
        Some(&bearer_header()),
        Some(&merge_body),
    )
    .await;

    assert_eq!(response.status(), StatusCode::OK);
    let body = response_json(response).await;
    assert_eq!(body["request"]["state"], "Resolved");
    assert_eq!(body["request"]["disposition"], "Accepted");

    let repo = find_repo(&fixture.state, TEST_REPO_OWNER, TEST_REPO_NAME)
        .await
        .unwrap();
    let live_tree = repo.live_tree();
    for (path, expected) in [
        ("/README.md", "hello from public request\n"),
        ("/ignored.txt", "tracked despite ignore\n"),
        ("/SECRET.md", "private\n"),
    ] {
        assert_eq!(
            blob_content(
                &fixture.state,
                live_tree.get(&ScopePath::parse(path).unwrap()).unwrap(),
            ),
            expected
        );
    }
}

#[tokio::test]
async fn public_request_merge_rejects_private_path_collision() {
    let fixture = public_merge_fixture(
        "req_private_collision",
        &[("SECRET.md", "public overwrite attempt\n")],
        false,
    )
    .await;

    let merge_body = format!(
        r#"{{"expected_main_oid":"{}","expected_head_oid":"{}"}}"#,
        fixture.raw_main_oid, fixture.request_head
    );
    let response = api_request(
        router(fixture.state.clone()),
        "POST",
        "/v1/repos/owner/repo/requests/req_private_collision/merge",
        Some(&bearer_header()),
        Some(&merge_body),
    )
    .await;

    assert_eq!(response.status(), StatusCode::CONFLICT);
    let repo = find_repo(&fixture.state, TEST_REPO_OWNER, TEST_REPO_NAME)
        .await
        .unwrap();
    let live_tree = repo.live_tree();
    let secret = live_tree
        .get(&ScopePath::parse("/SECRET.md").unwrap())
        .unwrap();
    assert_eq!(blob_content(&fixture.state, secret), "private\n");
}

async fn state_with_public_user() -> AppState {
    let state = test_state_with_repo_with_readme().await;
    let public_user = UserAccount {
        id: public_user_id(),
        handle: "public".to_string(),
        email: PUBLIC_EMAIL.to_string(),
        email_verified: true,
    };
    state
        .metadata
        .insert_user_for_tests(public_user)
        .await
        .unwrap();
    state
}

async fn test_state_with_repo_with_readme() -> AppState {
    let state = test_state_with_repo();
    cache_test_jwks(&state);
    state
        .metadata
        .replace_repository_for_tests(repo_with_readme(&state))
        .await
        .unwrap();
    state
}

async fn state_with_public_request() -> AppState {
    let state = state_with_public_user().await;
    grant_public_credits(&state, "ledger_grant").await;
    create_public_request(
        &state,
        "req_public",
        "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
        REQUEST_HEAD,
        "Public request",
        "ledger_stake",
        "event_created",
    )
    .await;
    state
}

fn repo_with_public_readme_and_private_secret(state: &AppState) -> StoredRepository {
    let mut repo = test_repo(&test_owner_id());
    repo.graph.commits.push(LogicalCommit {
        id: "rv1".to_string(),
        parent_ids: Vec::new(),
        author_id: repo.record.owner_user_id.clone(),
        author_visibility: AuthorVisibility::Visible,
        message: "initial".to_string(),
        changes: [
            (Visibility::Public, "/README.md", "hello\n"),
            (Visibility::Public, "/.gitignore", "ignored.txt\n"),
            (Visibility::Private, "/SECRET.md", "private\n"),
        ]
        .into_iter()
        .map(|(visibility, path, content)| FileChange {
            visibility,
            path: ScopePath::parse(path).unwrap(),
            old_content: None,
            new_content: Some(source_blob(state, content)),
        })
        .collect(),
    });
    repo
}

fn public_user_id() -> String {
    crate::db::scope_user_id_for_auth_identity("clerk", PUBLIC_SUBJECT)
}

async fn grant_public_credits(state: &AppState, ledger_entry_id: &str) {
    state
        .metadata
        .grant_user_credits(GrantUserCreditsInput {
            ledger_entry_id: ledger_entry_id.to_string(),
            user_id: public_user_id(),
            amount_credits: 20,
            now_unix: 1,
        })
        .await
        .unwrap();
}

async fn api_request(
    app: axum::Router,
    method: &str,
    uri: &str,
    bearer: Option<&str>,
    body: Option<&str>,
) -> Response {
    let mut request = Request::builder().method(method).uri(uri);
    if let Some(bearer) = bearer {
        request = request.header(AUTHORIZATION, bearer);
    }
    let body = match body {
        Some(json) => {
            request = request.header(CONTENT_TYPE, "application/json");
            Body::from(json.to_string())
        }
        None => Body::empty(),
    };
    app.oneshot(request.body(body).unwrap()).await.unwrap()
}

fn write_file(repo: &FsPath, path: &str, content: &str) {
    fs::write(repo.join(path), content).unwrap();
}
