use super::*;
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

#[tokio::test]
async fn repo_events_map_public_visibility_to_not_found_or_connected() {
    for public in [false, true] {
        let state = if public {
            test_state_with_readme().await
        } else {
            let state = test_state_with_repo();
            let mut repo = repo_with_readme(&state);
            repo.record.default_visibility = Visibility::Private;
            repo.policy = Policy::new(Visibility::Private);
            repo.graph.commits[0].changes[0].visibility = Visibility::Private;
            replace_test_repo(&state, repo).await;
            state
        };
        let response = events(state, None).await;
        assert_eq!(
            response.status(),
            if public {
                StatusCode::OK
            } else {
                StatusCode::NOT_FOUND
            }
        );
        if public {
            let mut stream = response.into_body().into_data_stream();
            let initial = next_event(&mut stream).await;
            assert!(initial.contains(r#""reason":"connected""#));
            assert!(initial.contains(r#""version":0"#));
        }
    }
}

#[tokio::test]
async fn repo_events_stream_permission_changes_to_members() {
    let state = test_state_with_repo();
    cache_test_jwks(&state);
    let writer_id = crate::db::scope_user_id_for_auth_identity("clerk", "user_writer");
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
            .contains(r#""reason":"connected""#)
    );

    state
        .metadata
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
        .repository_for_tests(TEST_REPO_ID)
        .await
        .unwrap()
        .unwrap()
        .record
        .change_version;
    state
        .publish_repo_change(TEST_REPO_ID, version, "visibility-changed")
        .await;

    let event = next_event(&mut stream).await;
    assert!(event.contains("event: repo-change"));
    assert!(event.contains(r#""reason":"visibility-changed""#));
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
            .contains(r#""reason":"connected""#)
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
