use super::*;

#[tokio::test]
async fn unrelated_public_reader_cannot_comment_on_public_request() {
    let state = state_with_public_request().await;
    let unrelated_user_id = crate::db::scope_user_id_for_auth_identity("clerk", "public_other");
    state
        .metadata
        .update(|catalog| {
            catalog.users.insert(
                unrelated_user_id.clone(),
                UserAccount {
                    id: unrelated_user_id,
                    handle: "public-other".to_string(),
                    email: "public-other@example.com".to_string(),
                    email_verified: true,
                },
            );
            Ok(())
        })
        .unwrap();
    let app = router(state.clone());

    let visible = app
        .clone()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/v1/repos/owner/repo/requests/req_public")
                .header(
                    AUTHORIZATION,
                    bearer_header_for("public_other", "public-other@example.com"),
                )
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(visible.status(), StatusCode::OK);
    let body = response_json(visible).await;
    assert_eq!(body["request"]["permissions"]["can_comment"], false);

    let comment = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/repos/owner/repo/requests/req_public/comments")
                .header(
                    AUTHORIZATION,
                    bearer_header_for("public_other", "public-other@example.com"),
                )
                .header(CONTENT_TYPE, "application/json")
                .body(Body::from(r#"{"body":"drive-by comment"}"#))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(comment.status(), StatusCode::FORBIDDEN);
    state
        .metadata
        .read(|catalog| {
            assert_eq!(catalog.request_events.len(), 1);
            Ok(())
        })
        .await
        .unwrap();
}

#[tokio::test]
async fn maintainer_can_add_and_remove_public_request_editor() {
    let state = state_with_public_request().await;
    let editor_subject = "public_editor";
    let editor_email = "public-editor@example.com";
    let editor_user_id = crate::db::scope_user_id_for_auth_identity("clerk", editor_subject);
    insert_public_user(&state, &editor_user_id, "public-editor", editor_email);
    let app = router(state.clone());

    let add = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/repos/owner/repo/requests/req_public/editors")
                .header(AUTHORIZATION, bearer_header())
                .header(CONTENT_TYPE, "application/json")
                .body(Body::from(format!(r#"{{"user_id":"{editor_user_id}"}}"#)))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(add.status(), StatusCode::OK);
    let add_body = response_json(add).await;
    assert!(
        add_body["request"]["editor_user_ids"]
            .as_array()
            .unwrap()
            .iter()
            .any(|value| value == &serde_json::json!(editor_user_id))
    );

    let editor_view = app
        .clone()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/v1/repos/owner/repo/requests/req_public")
                .header(
                    AUTHORIZATION,
                    bearer_header_for(editor_subject, editor_email),
                )
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(editor_view.status(), StatusCode::OK);
    let editor_view_body = response_json(editor_view).await;
    assert_eq!(
        editor_view_body["request"]["permissions"]["can_comment"],
        true
    );
    assert_eq!(
        editor_view_body["request"]["permissions"]["can_pull_branch"],
        true
    );
    assert_eq!(
        editor_view_body["request"]["permissions"]["can_push_branch"],
        true
    );

    let comment = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/repos/owner/repo/requests/req_public/comments")
                .header(
                    AUTHORIZATION,
                    bearer_header_for(editor_subject, editor_email),
                )
                .header(CONTENT_TYPE, "application/json")
                .body(Body::from(r#"{"body":"I can help with this."}"#))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(comment.status(), StatusCode::OK);

    let remove = app
        .clone()
        .oneshot(
            Request::builder()
                .method("DELETE")
                .uri(format!(
                    "/v1/repos/owner/repo/requests/req_public/editors/{editor_user_id}"
                ))
                .header(AUTHORIZATION, bearer_header())
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(remove.status(), StatusCode::OK);
    let remove_body = response_json(remove).await;
    assert_eq!(
        remove_body["request"]["editor_user_ids"]
            .as_array()
            .unwrap()
            .len(),
        0
    );

    let editor_view_after_remove = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/v1/repos/owner/repo/requests/req_public")
                .header(
                    AUTHORIZATION,
                    bearer_header_for(editor_subject, editor_email),
                )
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(editor_view_after_remove.status(), StatusCode::OK);
    let editor_view_after_remove_body = response_json(editor_view_after_remove).await;
    assert_eq!(
        editor_view_after_remove_body["request"]["permissions"]["can_comment"],
        false
    );
    assert_eq!(
        editor_view_after_remove_body["request"]["permissions"]["can_pull_branch"],
        false
    );
    assert_eq!(
        editor_view_after_remove_body["request"]["permissions"]["can_push_branch"],
        false
    );
}

fn insert_public_user(state: &AppState, user_id: &str, handle: &str, email: &str) {
    let user = UserAccount {
        id: user_id.to_string(),
        handle: handle.to_string(),
        email: email.to_string(),
        email_verified: true,
    };
    state
        .metadata
        .update(|catalog| {
            catalog.users.insert(user.id.clone(), user);
            Ok(())
        })
        .unwrap();
}
