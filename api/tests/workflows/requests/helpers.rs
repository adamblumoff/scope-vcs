use super::*;
use crate::domain::requests::{
    RecordRequestRevisionInput, RequestActorRole, RequestAudience, StartRequestInput,
    SubmitRequestInput, canonical_request_ref,
};

pub(super) struct PublicMergeFixture {
    pub state: AppState,
    pub raw_main_oid: String,
    pub request_head: String,
    _request_repo: TempGitRepo,
}

pub(super) async fn public_merge_fixture(
    request_id: &str,
    request_files: &[(&str, &str)],
    track_ignored_file: bool,
) -> PublicMergeFixture {
    let state = state_with_public_user().await;
    cache_test_jwks(&state);
    grant_public_credits(&state, &format!("ledger_{request_id}_grant")).await;

    let raw_repo = temp_git_repo(&format!("{request_id}-raw"));
    for (path, content) in [("README.md", "hello\n"), ("SECRET.md", "private\n")] {
        write_file(&raw_repo, path, content);
    }
    if track_ignored_file {
        write_file(&raw_repo, ".gitignore", "ignored.txt\n");
    }
    run_git(Some(&raw_repo), &["add", "."], "stage raw main").unwrap();
    commit_all(&raw_repo, "initial raw main");
    let raw_main_oid = git_head_oid(&raw_repo);
    let raw_snapshot =
        git_snapshot_from_ref(&state, TEST_REPO_ID, &raw_repo, "refs/heads/main").unwrap();
    let mut repo = repo_with_public_readme_and_private_secret(&state);
    repo.git_snapshot = Some(raw_snapshot);
    replace_test_repo(&state, repo).await;

    let repo = find_repo(&state, TEST_REPO_OWNER, TEST_REPO_NAME)
        .await
        .unwrap();
    let projection = project_graph(
        &repo.policy,
        &repo.graph,
        &repo.visibility_events,
        ProjectionViewKey::Public,
    );
    let public_repo = projection_bare_repo_for_state(&state, &projection).unwrap();
    let public_main_oid = git_head_oid(&public_repo);
    let request_repo =
        clone_test_repo(&public_repo, &format!("{request_id}-public-request"), false);
    for (path, content) in request_files {
        write_file(&request_repo, path, content);
    }
    run_git(Some(&request_repo), &["add", "."], "stage public request").unwrap();
    if track_ignored_file {
        run_git(
            Some(&request_repo),
            &["add", "-f", "ignored.txt"],
            "stage ignored file",
        )
        .unwrap();
    }
    commit_all(&request_repo, "public request change");
    let request_head = git_head_oid(&request_repo);
    let request_name = request_name(request_id);
    let request_ref = canonical_request_ref(&request_name);
    run_git(
        Some(&request_repo),
        &["update-ref", &request_ref, &request_head],
        "create public request ref",
    )
    .unwrap();
    let request_snapshot =
        git_snapshot_from_ref(&state, TEST_REPO_ID, &request_repo, &request_ref).unwrap();
    create_public_request(
        &state,
        request_id,
        &public_main_oid,
        &request_head,
        "Public request merge",
        &format!("ledger_{request_id}_stake"),
        &format!("event_{request_id}_created"),
    )
    .await;
    state
        .metadata
        .record_request_revision(RecordRequestRevisionInput {
            request_id: request_id.to_string(),
            actor_user_id: public_user_id(),
            actor_can_edit: true,
            expected_old_head_oid: Some(request_head.clone()),
            new_head_oid: request_head.clone(),
            git_snapshot: Some(request_snapshot),
            event_id: format!("event_{request_id}_revision"),
            body: None,
            now_unix: 3,
        })
        .await
        .unwrap();
    PublicMergeFixture {
        state,
        raw_main_oid,
        request_head,
        _request_repo: request_repo,
    }
}

pub(super) async fn create_public_request(
    state: &AppState,
    request_id: &str,
    base_main_oid: &str,
    head_oid: &str,
    title: &str,
    stake_ledger_entry_id: &str,
    event_id: &str,
) {
    create_request(RequestFixture {
        state,
        request_id,
        author_user_id: public_user_id(),
        title,
        role: RequestActorRole::Public,
        audience: RequestAudience::Public,
        base_main_oid,
        head_oid,
        stake_credits: 10,
        stake_ledger_entry_id: Some(stake_ledger_entry_id),
        event_id,
        snapshot: "public request git snapshot",
    })
    .await;
}

pub(crate) async fn create_owner_request(state: &AppState, request_id: &str, head_oid: &str) {
    let event_id = format!("event_created_{request_id}");
    create_request(RequestFixture {
        state,
        request_id,
        author_user_id: test_owner_id(),
        title: "Owner request",
        role: RequestActorRole::Owner,
        audience: RequestAudience::Private,
        base_main_oid: "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        head_oid,
        stake_credits: 0,
        stake_ledger_entry_id: None,
        event_id: &event_id,
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
    stake_credits: u32,
    stake_ledger_entry_id: Option<&'a str>,
    event_id: &'a str,
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
            now_unix: 2,
        })
        .await
        .unwrap();
    fixture
        .state
        .metadata
        .record_working_request_upload(RecordWorkingRequestUploadInput {
            request_id: fixture.request_id.to_string(),
            actor_user_id: fixture.author_user_id.clone(),
            actor_can_edit: true,
            expected_old_head_oid: None,
            new_head_oid: fixture.head_oid.to_string(),
            git_snapshot: source_blob(fixture.state, fixture.snapshot),
            now_unix: 3,
        })
        .await
        .unwrap();
    fixture
        .state
        .metadata
        .submit_request(SubmitRequestInput {
            request_id: fixture.request_id.to_string(),
            actor_user_id: fixture.author_user_id,
            expected_head_oid: fixture.head_oid.to_string(),
            stake_credits: fixture.stake_credits,
            stake_ledger_entry_id: fixture.stake_ledger_entry_id.map(str::to_string),
            event_id: fixture.event_id.to_string(),
            now_unix: 4,
        })
        .await
        .unwrap();
}

pub(super) async fn start_request_via_http(app: axum::Router, bearer: &str) -> serde_json::Value {
    let response = api_request(
        app,
        "POST",
        "/v1/repos/owner/repo/requests",
        Some(bearer),
        Some(r#"{"name":"fix-parser-crash","audience":"Public"}"#),
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    response_json(response).await
}

fn request_name(request_id: &str) -> String {
    request_id.replace('_', "-")
}

pub(super) async fn submit_request_via_http(
    app: axum::Router,
    bearer: &str,
    request_id: &str,
    body: &str,
) -> Response {
    api_request(
        app,
        "POST",
        &format!("/v1/repos/owner/repo/requests/{request_id}/submit"),
        Some(bearer),
        Some(body),
    )
    .await
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
            actor_user_id: state
                .metadata
                .request_by_id(request_id)
                .await
                .unwrap()
                .unwrap()
                .author_user_id,
            actor_can_edit: true,
            expected_old_head_oid: None,
            new_head_oid: head_oid.to_string(),
            git_snapshot: source_blob(state, "working request git snapshot"),
            now_unix: 2,
        })
        .await
        .unwrap();
}
