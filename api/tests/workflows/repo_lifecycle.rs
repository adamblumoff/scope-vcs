use super::*;

async fn request(
    state: AppState,
    method: &str,
    uri: impl AsRef<str>,
    authorization: Option<String>,
    body: Option<String>,
) -> Response {
    let mut request = Request::builder().method(method).uri(uri.as_ref());
    if let Some(authorization) = authorization {
        request = request.header(AUTHORIZATION, authorization);
    }
    let body = if let Some(body) = body {
        request = request.header(CONTENT_TYPE, "application/json");
        Body::from(body)
    } else {
        Body::empty()
    };
    router(state)
        .oneshot(request.body(body).unwrap())
        .await
        .unwrap()
}

fn pending_invite(
    id: &str,
    token_hash: String,
    email: &str,
    expires_at: u64,
    lifetime_secs: u64,
) -> RepositoryInvite {
    let created_at = expires_at.saturating_sub(lifetime_secs);
    RepositoryInvite {
        id: id.to_string(),
        repo_id: TEST_REPO_ID.to_string(),
        invited_email: email.to_string(),
        invited_email_normalized: crate::domain::store::normalize_repository_invite_email(email),
        permissions: RepositoryMemberPermissions::default(),
        invited_by_user_id: test_owner_id(),
        state: RepositoryInviteState::Pending,
        token_hash,
        created_at_unix: created_at,
        updated_at_unix: created_at,
        expires_at_unix: expires_at,
        accepted_by_user_id: None,
        accepted_at_unix: None,
        revoked_at_unix: None,
    }
}

async fn seed_invite(
    state: &AppState,
    id: &str,
    token: &str,
    email: &str,
    expires_at: u64,
) -> String {
    let token_hash = repository_invite_token_hash(token);
    let invite = pending_invite(id, token_hash.clone(), email, expires_at, 600);
    state
        .metadata
        .mutate_repository_for_tests(TEST_REPO_ID, move |repo| repo.invitations.push(invite))
        .await
        .unwrap();
    token_hash
}

#[tokio::test]
async fn create_repo_route_creates_user_and_lists_repo() {
    let state = test_state_with_jwks();
    let response = request(
        state.clone(),
        "POST",
        "/v1/repos",
        Some(bearer_header()),
        Some(r#"{"name":"Scope_App"}"#.to_string()),
    )
    .await;

    assert_eq!(response.status(), StatusCode::OK);
    let body = response_json(response).await;
    assert_eq!(body["repo"]["id"], "owner/scope_app");
    assert_eq!(body["repo"]["access"]["actor"], "Owner");
    assert_eq!(
        body["init"]["git_remote_url"],
        "http://localhost:8080/git/permissioned/owner/scope_app"
    );
    let secret = body["init"]["token"]["secret"].as_str().unwrap();
    assert!(secret.starts_with("scope_fp_"));
    let push_secret = body["init"]["push_token"]["secret"].as_str().unwrap();
    assert!(push_secret.starts_with("scope_git_"));

    let response = request(
        state.clone(),
        "GET",
        "/v1/repos",
        Some(bearer_header()),
        None,
    )
    .await;

    assert_eq!(response.status(), StatusCode::OK);
    let body = response_json(response).await;
    assert_eq!(body[0]["id"], "owner/scope_app");
}

#[tokio::test]
async fn invite_acceptance_returns_member_access() {
    let state = test_state_with_repo();
    cache_test_jwks(&state);
    let invited_email = "invitee@example.com";
    let create_response = request(
        state.clone(),
        "POST",
        "/v1/repos/owner/repo/invites",
        Some(bearer_header()),
        Some(
            serde_json::json!({
                "email": invited_email,
                "permissions": RepositoryMemberPermissions::default(),
            })
            .to_string(),
        ),
    )
    .await;
    assert_eq!(create_response.status(), StatusCode::OK);
    let create_body = response_json(create_response).await;
    let invite_url = create_body["invite_url"].as_str().unwrap();
    let token = invite_url.rsplit('/').next().unwrap();

    let accept_response = request(
        state,
        "POST",
        format!("/v1/repository-invites/{token}/accept"),
        Some(bearer_header_for("user_invitee", invited_email)),
        None,
    )
    .await;

    assert_eq!(accept_response.status(), StatusCode::OK);
    let body = response_json(accept_response).await;
    assert_eq!(body["repo"]["access"]["actor"], "Member");
}

#[tokio::test]
async fn owner_can_revoke_pending_invite_before_acceptance() {
    let state = test_state_with_repo();
    cache_test_jwks(&state);
    let token = "revoked-invite-token";
    let invited_email = "invitee@example.com";
    seed_invite(
        &state,
        "invite_revoke",
        token,
        invited_email,
        unix_now() + 600,
    )
    .await;

    let revoke_response = request(
        state.clone(),
        "DELETE",
        "/v1/repos/owner/repo/invites/invite_revoke",
        Some(bearer_header()),
        None,
    )
    .await;

    assert_eq!(revoke_response.status(), StatusCode::OK);
    let body = response_json(revoke_response).await;
    assert_eq!(body["state"], "Revoked");

    let accept_response = request(
        state.clone(),
        "POST",
        format!("/v1/repository-invites/{token}/accept"),
        Some(bearer_header_for("user_invitee", invited_email)),
        None,
    )
    .await;

    assert_eq!(accept_response.status(), StatusCode::CONFLICT);
}
