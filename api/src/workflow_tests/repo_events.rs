use super::*;
use crate::repo_events::RepoChangeReason;
use scope_api_contract::RunChangeKind;
use scope_domain::requests::{RequestActorRole, RequestAudience, StartRequestInput};
use std::time::Duration;
use tokio_stream::StreamExt;

async fn events(state: AppState, auth: Option<String>) -> Response {
    let mut request = Request::builder()
        .method("GET")
        .uri("/v1/repos/owner/repo/events");
    if let Some(auth) = auth {
        request = request.header(AUTHORIZATION, auth);
    }
    router(state)
        .oneshot(request.body(Body::empty()).unwrap())
        .await
        .unwrap()
}

async fn next_event(stream: &mut axum::body::BodyDataStream) -> String {
    let bytes = tokio::time::timeout(Duration::from_secs(5), stream.next())
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    String::from_utf8(bytes.to_vec()).unwrap()
}

async fn next_repo_change_event(
    stream: &mut axum::body::BodyDataStream,
    expected_version: u64,
) -> String {
    let expected_version = format!(r#""version":{expected_version}"#);
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let bytes = stream.next().await.unwrap().unwrap();
            let event = String::from_utf8(bytes.to_vec()).unwrap();
            if event.contains("event: repo-change") && event.contains(&expected_version) {
                return event;
            }
        }
    })
    .await
    .unwrap()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn real_git_push_emits_one_post_commit_event_and_failed_push_emits_none() {
    let secret = "scope_git_sse_test";
    let state = test_state_with_git_push_token(secret).await;
    cache_test_jwks(&state);
    let (origin, _server) = spawn_test_server(&state).await;
    let response = events(state.clone(), Some(bearer_header())).await;
    assert_eq!(response.status(), StatusCode::OK);
    let mut stream = response.into_body().into_data_stream();
    assert!(
        next_event(&mut stream)
            .await
            .contains(r#""kind":"Connected""#)
    );

    let permissioned_remote = format!("{origin}/git/permissioned/{TEST_REPO_ID}").replacen(
        "http://",
        &format!("http://scope:{secret}@"),
        1,
    );
    let public_remote = format!("{origin}/git/public/{TEST_REPO_ID}");
    let source = TempGitRepo(unique_test_path("sse-real-push"));
    run_git(
        None,
        &["clone", &public_remote, source.to_str().unwrap()],
        "clone repository for SSE push",
    )
    .unwrap();
    run_git(
        Some(&source),
        &["remote", "set-url", "origin", &permissioned_remote],
        "point SSE fixture at permissioned remote",
    )
    .unwrap();
    fs::write(source.join("README.html"), "<h1>SSE push</h1>\n").unwrap();
    run_git(
        Some(&source),
        &["add", "README.html"],
        "stage SSE landing file",
    )
    .unwrap();
    commit_all(&source, "add landing file through real push");
    configure_push_intent_header(&state, &source, &permissioned_remote, &test_owner_id()).await;
    run_git(
        Some(&source),
        &["push", "origin", "HEAD:main"],
        "push landing file for SSE event",
    )
    .unwrap();

    let version = find_repo(&state, TEST_REPO_OWNER, TEST_REPO_NAME)
        .await
        .unwrap()
        .record
        .change_version;
    let event = next_repo_change_event(&mut stream, version).await;
    assert!(event.contains(r#""reason":"push-received""#), "{event}");
    assert!(event.contains(&format!(r#""version":{version}"#)));
    assert!(
        tokio::time::timeout(
            Duration::from_millis(250),
            next_repo_change_event(&mut stream, version)
        )
        .await
        .is_err()
    );

    fs::write(source.join("README.html"), "<h1>stale intent</h1>\n").unwrap();
    run_git(
        Some(&source),
        &["add", "README.html"],
        "stage stale-intent landing file",
    )
    .unwrap();
    commit_all(&source, "prepare stale push intent");
    let header_key = format!("http.{permissioned_remote}.extraHeader");
    run_git(
        Some(&source),
        &["config", "--unset-all", &header_key],
        "clear previous push intent",
    )
    .unwrap();
    configure_push_intent_header(&state, &source, &permissioned_remote, &test_owner_id()).await;
    fs::write(source.join("README.html"), "<h1>different head</h1>\n").unwrap();
    run_git(
        Some(&source),
        &["add", "README.html"],
        "change head after minting push intent",
    )
    .unwrap();
    run_git(
        Some(&source),
        &[
            "-c",
            "user.name=Scope Test",
            "-c",
            "user.email=scope-test@example.test",
            "commit",
            "--amend",
            "--no-edit",
        ],
        "invalidate push intent head",
    )
    .unwrap();
    let failed = run_git_output(
        Some(&source),
        &["push", "origin", "HEAD:main"],
        "reject stale-intent push",
    )
    .unwrap();
    assert!(!failed.status.success());
    assert!(
        tokio::time::timeout(
            Duration::from_millis(250),
            next_repo_change_event(&mut stream, version + 1)
        )
        .await
        .is_err()
    );
}

#[tokio::test]
async fn repo_events_stay_private_when_only_canonical_rules_are_public() {
    let state = test_state_with_repo();
    let mut repo = repo_with_readme(&state);
    repo.repo_config = repo_config(Visibility::Private);
    repo.policy = Policy::new(Visibility::Private);
    repo.policy
        .add_rule(VisibilityRule::public(
            ScopePath::parse("/.scope/RULES.md").unwrap(),
        ))
        .unwrap();
    repo.graph.commits[0].changes[0].visibility = Visibility::Private;
    replace_test_repo(&state, repo).await;

    let response = events(state, None).await;

    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn repo_events_stream_permission_changes_to_members() {
    let state = test_state_with_repo();
    cache_test_jwks(&state);
    let writer_id = scope_postgres::db::scope_user_id_for_auth_identity("clerk", "user_writer");
    let mut repo = repo_with_readme(&state);
    repo.members.push(test_repository_member(
        TEST_REPO_ID,
        writer_id.clone(),
        member_permissions(true, false, false),
    ));
    replace_test_repo(&state, repo).await;
    let response = events(
        state.clone(),
        Some(bearer_header_for("user_writer", "writer@example.com")),
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let mut stream = response.into_body().into_data_stream();
    assert!(
        next_event(&mut stream)
            .await
            .contains(r#""kind":"Connected""#)
    );

    state
        .metadata
        .repositories()
        .mutate_repository_for_tests(TEST_REPO_ID, |repo| {
            repo.members
                .iter_mut()
                .find(|member| member.user_id == writer_id)
                .unwrap()
                .permissions
                .can_push = false;
            repo.bump_change_version();
        })
        .await
        .unwrap();
    let version = state
        .metadata
        .repositories()
        .repository_for_tests(TEST_REPO_ID)
        .await
        .unwrap()
        .unwrap()
        .record
        .change_version;
    state
        .publish_repo_change(TEST_REPO_ID, version, RepoChangeReason::VisibilityChanged)
        .await;

    let event = next_event(&mut stream).await;
    assert!(event.contains("event: repo-change"));
    assert!(event.contains(r#""reason":"visibility-changed"#));
    assert!(event.contains(&format!(r#""version":{version}"#)));
}

#[tokio::test]
async fn repo_events_close_when_repo_is_deleted() {
    let state = test_state_with_readme().await;
    cache_test_jwks(&state);
    let app = router(state.clone());
    let response = events(state, Some(bearer_header())).await;
    assert_eq!(response.status(), StatusCode::OK);
    let mut stream = response.into_body().into_data_stream();
    assert!(
        next_event(&mut stream)
            .await
            .contains(r#""kind":"Connected""#)
    );

    let deleted = app
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
    assert_eq!(deleted.status(), StatusCode::OK);
    assert!(
        tokio::time::timeout(Duration::from_secs(5), stream.next())
            .await
            .unwrap()
            .is_none()
    );
}

#[tokio::test]
async fn public_repo_stream_drops_private_discussion_identifiers() {
    let state = test_state_with_readme().await;
    state
        .metadata
        .requests()
        .start_request(StartRequestInput {
            id: "req_private_stream".to_string(),
            repo_id: TEST_REPO_ID.to_string(),
            name: "private-stream".to_string(),
            author_user_id: test_owner_id(),
            title: Some("Private stream".to_string()),
            author_role: RequestActorRole::Owner,
            audience: RequestAudience::Private,
            base_main_oid: "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".to_string(),
            event_id: "event_private_stream_started".to_string(),
            now_unix: 1,
        })
        .await
        .unwrap();
    state
        .metadata
        .requests()
        .start_request(StartRequestInput {
            id: "req_public_stream".to_string(),
            repo_id: TEST_REPO_ID.to_string(),
            name: "public-stream".to_string(),
            author_user_id: test_owner_id(),
            title: Some("Public stream".to_string()),
            author_role: RequestActorRole::Owner,
            audience: RequestAudience::Public,
            base_main_oid: "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".to_string(),
            event_id: "event_public_stream_started".to_string(),
            now_unix: 1,
        })
        .await
        .unwrap();
    state
        .metadata
        .requests()
        .mutate_request_for_tests("req_public_stream", |request| {
            request.submitted_at_unix = Some(2);
            request.updated_at_unix = 2;
        })
        .await
        .unwrap();

    let response = events(state.clone(), None).await;
    assert_eq!(response.status(), StatusCode::OK);
    let mut stream = response.into_body().into_data_stream();
    assert!(
        next_event(&mut stream)
            .await
            .contains(r#""kind":"Connected""#)
    );

    state
        .publish_request_timeline_change(
            TEST_REPO_ID,
            "req_private_stream".to_string(),
            "discussion_private".to_string(),
            2,
            RequestAudience::Private,
        )
        .await;
    assert!(
        tokio::time::timeout(std::time::Duration::from_millis(250), stream.next())
            .await
            .is_err()
    );

    state
        .publish_request_timeline_change(
            TEST_REPO_ID,
            "req_public_stream".to_string(),
            "discussion_ready".to_string(),
            2,
            RequestAudience::Public,
        )
        .await;
    let ready = next_event(&mut stream).await;
    assert!(ready.contains("discussion_ready"));

    state
        .publish_request_timeline_change(
            TEST_REPO_ID,
            "req_public_stream".to_string(),
            "discussion_open".to_string(),
            3,
            RequestAudience::Public,
        )
        .await;
    let visible = next_event(&mut stream).await;
    assert!(visible.contains("req_public_stream"));
    assert!(visible.contains("discussion_open"));
    assert!(!visible.contains("req_private_stream"));
}

#[tokio::test]
async fn run_changes_are_visible_to_members_and_hidden_from_public_repo_streams() {
    let state = test_state_with_readme().await;
    cache_test_jwks(&state);
    let public = events(state.clone(), None).await;
    let member = events(state.clone(), Some(bearer_header())).await;
    assert_eq!(public.status(), StatusCode::OK);
    assert_eq!(member.status(), StatusCode::OK);
    let mut public_stream = public.into_body().into_data_stream();
    let mut member_stream = member.into_body().into_data_stream();
    assert!(next_event(&mut public_stream).await.contains("Connected"));
    assert!(next_event(&mut member_stream).await.contains("Connected"));

    state
        .publish_run_change(
            TEST_REPO_ID,
            "run_private".to_string(),
            RunChangeKind::Created,
        )
        .await;

    let member_event = next_event(&mut member_stream).await;
    assert!(member_event.contains(r#""RunChanged""#));
    assert!(member_event.contains(r#""run_id":"run_private""#));
    assert!(member_event.contains(r#""change":"Created""#));
    assert!(
        tokio::time::timeout(Duration::from_millis(250), public_stream.next())
            .await
            .is_err()
    );
}
