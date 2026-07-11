use super::*;

#[tokio::test]
async fn maintainer_can_add_and_remove_public_request_editor() {
    let state = state_with_public_request().await;
    let editor_subject = "public_editor";
    let editor_email = "public-editor@example.com";
    let editor_user_id = crate::db::scope_user_id_for_auth_identity("clerk", editor_subject);
    insert_public_user(&state, &editor_user_id, "public-editor", editor_email).await;
    let app = router(state.clone());
    let editor_bearer = bearer_header_for(editor_subject, editor_email);
    let owner_bearer = bearer_header();

    let add_body = format!(r#"{{"user_id":"{editor_user_id}"}}"#);
    let add = api_request(
        app.clone(),
        "POST",
        "/v1/repos/owner/repo/requests/req_public/editors",
        Some(&owner_bearer),
        Some(&add_body),
    )
    .await;
    let status = add.status();
    let add_body = response_json(add).await;
    assert_eq!(status, StatusCode::OK, "{add_body}");
    assert!(
        add_body["request"]["editor_user_ids"]
            .as_array()
            .unwrap()
            .iter()
            .any(|value| value == &serde_json::json!(editor_user_id))
    );

    let editor_view = api_request(
        app.clone(),
        "GET",
        "/v1/repos/owner/repo/requests/req_public",
        Some(&editor_bearer),
        None,
    )
    .await;
    assert_eq!(editor_view.status(), StatusCode::OK);
    let editor_view_body = response_json(editor_view).await;
    assert_eq!(
        editor_view_body["request"]["permissions"]["can_comment"],
        true
    );

    let comment = api_request(
        app.clone(),
        "POST",
        "/v1/repos/owner/repo/requests/req_public/comments",
        Some(&editor_bearer),
        Some(r#"{"body":"I can help with this."}"#),
    )
    .await;
    assert_eq!(comment.status(), StatusCode::OK);

    let remove = api_request(
        app.clone(),
        "DELETE",
        &format!("/v1/repos/owner/repo/requests/req_public/editors/{editor_user_id}"),
        Some(&owner_bearer),
        None,
    )
    .await;
    assert_eq!(remove.status(), StatusCode::OK);
    let remove_body = response_json(remove).await;
    assert_eq!(
        remove_body["request"]["editor_user_ids"]
            .as_array()
            .unwrap()
            .len(),
        0
    );

    let editor_view_after_remove = api_request(
        app.clone(),
        "GET",
        "/v1/repos/owner/repo/requests/req_public",
        Some(&editor_bearer),
        None,
    )
    .await;
    assert_eq!(editor_view_after_remove.status(), StatusCode::OK);
    let editor_view_after_remove_body = response_json(editor_view_after_remove).await;
    assert_eq!(
        editor_view_after_remove_body["request"]["permissions"]["can_comment"],
        false
    );
    let comment = api_request(
        app,
        "POST",
        "/v1/repos/owner/repo/requests/req_public/comments",
        Some(&editor_bearer),
        Some(r#"{"body":"no longer allowed"}"#),
    )
    .await;
    assert_eq!(comment.status(), StatusCode::FORBIDDEN);
}

async fn insert_public_user(state: &AppState, user_id: &str, handle: &str, email: &str) {
    let user = UserAccount {
        id: user_id.to_string(),
        handle: handle.to_string(),
        email: email.to_string(),
        email_verified: true,
    };
    state.metadata.insert_user_for_tests(user).await.unwrap();
}
