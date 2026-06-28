use super::*;
use tokio_stream::StreamExt;

#[tokio::test]
async fn repo_events_streams_committed_visibility_changes() {
    let state = test_state_with_repo();
    cache_test_jwks(&state);
    {
        let mut catalog = lock_catalog(&state).unwrap();
        catalog
            .repositories
            .insert(TEST_REPO_ID.to_string(), repo_with_readme());
    }
    let app = router(state.clone());

    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/v1/repos/owner/repo/events")
                .header(AUTHORIZATION, bearer_header())
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let mut stream = response.into_body().into_data_stream();
    let initial = stream.next().await.unwrap().unwrap();
    let initial = String::from_utf8(initial.to_vec()).unwrap();
    assert!(initial.contains(r#""reason":"connected""#));
    assert!(initial.contains(r#""version":1"#));

    let update_response = app
        .oneshot(
            Request::builder()
                .method("PATCH")
                .uri("/v1/repos/owner/repo/files/visibility")
                .header(AUTHORIZATION, bearer_header())
                .header(CONTENT_TYPE, "application/json")
                .body(Body::from(
                    r#"{"paths":["/README.md"],"visibility":"Private"}"#,
                ))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(update_response.status(), StatusCode::OK);
    let event = tokio::time::timeout(std::time::Duration::from_secs(1), stream.next())
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    let event = String::from_utf8(event.to_vec()).unwrap();
    assert!(event.contains("event: repo-change"));
    assert!(event.contains(r#""reason":"visibility-changed""#));
    assert!(event.contains(r#""version":2"#));
}

#[tokio::test]
async fn repo_events_hide_unreadable_private_repo() {
    let state = test_state_with_repo();
    {
        let mut repo = repo_with_readme();
        repo.record.default_visibility = Visibility::Private;
        repo.policy = Policy::new(Visibility::Private, &repo.record.owner_user_id);
        repo.graph.commits[0].changes[0].visibility = Visibility::Private;
        let mut catalog = lock_catalog(&state).unwrap();
        catalog.repositories.insert(TEST_REPO_ID.to_string(), repo);
    }

    let response = router(state)
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/v1/repos/owner/repo/events")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn repo_events_reject_public_reader_for_public_repo() {
    let state = test_state_with_repo();
    {
        let mut catalog = lock_catalog(&state).unwrap();
        catalog
            .repositories
            .insert(TEST_REPO_ID.to_string(), repo_with_readme());
    }

    let response = router(state)
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/v1/repos/owner/repo/events")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn repo_events_close_when_write_access_is_revoked() {
    let state = test_state_with_repo();
    cache_test_jwks(&state);
    let writer_clerk_id = "user_writer";
    let writer_email = "writer@example.com";
    let writer_id = crate::db::scope_user_id_for_auth_identity("clerk", writer_clerk_id);
    {
        let mut repo = repo_with_readme();
        repo.memberships.push(RepoMembership {
            repo_id: TEST_REPO_ID.to_string(),
            user_id: writer_id.clone(),
            role: RepoRole::Writer,
        });
        let mut catalog = lock_catalog(&state).unwrap();
        catalog.repositories.insert(TEST_REPO_ID.to_string(), repo);
    }
    let app = router(state.clone());

    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/v1/repos/owner/repo/events")
                .header(
                    AUTHORIZATION,
                    bearer_header_for(writer_clerk_id, writer_email),
                )
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let mut stream = response.into_body().into_data_stream();
    let initial = stream.next().await.unwrap().unwrap();
    let initial = String::from_utf8(initial.to_vec()).unwrap();
    assert!(initial.contains(r#""reason":"connected""#));

    let change_version = {
        let mut catalog = lock_catalog(&state).unwrap();
        let repo = catalog.repositories.get_mut(TEST_REPO_ID).unwrap();
        repo.memberships
            .iter_mut()
            .find(|membership| membership.user_id == writer_id)
            .unwrap()
            .role = RepoRole::Reader;
        repo.bump_change_version();
        repo.record.change_version
    };
    state.publish_repo_change(TEST_REPO_ID, change_version, "visibility-changed");

    let closed = tokio::time::timeout(std::time::Duration::from_secs(1), stream.next())
        .await
        .unwrap();
    assert!(closed.is_none());
}

#[tokio::test]
async fn repo_events_close_when_repo_is_deleted() {
    let state = test_state_with_repo();
    cache_test_jwks(&state);
    {
        let mut catalog = lock_catalog(&state).unwrap();
        catalog
            .repositories
            .insert(TEST_REPO_ID.to_string(), repo_with_readme());
    }
    let app = router(state);

    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/v1/repos/owner/repo/events")
                .header(AUTHORIZATION, bearer_header())
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let mut stream = response.into_body().into_data_stream();
    let initial = stream.next().await.unwrap().unwrap();
    let initial = String::from_utf8(initial.to_vec()).unwrap();
    assert!(initial.contains(r#""reason":"connected""#));

    let delete_response = app
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

    assert_eq!(delete_response.status(), StatusCode::OK);
    let closed = tokio::time::timeout(std::time::Duration::from_secs(1), stream.next())
        .await
        .unwrap();
    assert!(closed.is_none());
}
