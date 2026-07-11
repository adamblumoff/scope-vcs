use super::*;

async fn repo_with_secret(state: &AppState, path: &str) {
    let mut repo = repo_with_readme(state);
    repo.policy
        .add_rule(VisibilityRule::private(ScopePath::parse(path).unwrap()))
        .unwrap();
    repo.graph.commits[0].changes.push(FileChange {
        visibility: Visibility::Private,
        path: ScopePath::parse(path).unwrap(),
        old_content: None,
        new_content: Some(source_blob(state, "owner only")),
    });
    replace_test_repo(state, repo).await;
}

fn auth_headers(value: impl AsRef<str>) -> HeaderMap {
    let mut headers = HeaderMap::new();
    headers.insert(AUTHORIZATION, value.as_ref().parse().unwrap());
    headers
}

async fn cli_basic_headers(state: &AppState) -> HeaderMap {
    let grant = state
        .metadata
        .create_cli_exchange_grant(&test_user(
            test_owner_id(),
            TEST_REPO_OWNER,
            TEST_OWNER_EMAIL,
        ))
        .await
        .unwrap();
    let token = state
        .metadata
        .exchange_cli_grant(&grant.exchange_token)
        .await
        .unwrap()
        .session_token;
    auth_headers(format!("Basic {}", BASE64.encode(format!("scope:{token}"))))
}

#[tokio::test]
async fn permissioned_git_projection_accepts_basic_scope_cli_session_for_repo_owner() {
    let state = test_state_with_repo();
    repo_with_secret(&state, "/secret.txt").await;
    let headers = cli_basic_headers(&state).await;

    let projection = git_projection_for_request(
        &state,
        &headers,
        TEST_REPO_OWNER,
        TEST_REPO_NAME,
        GitRemoteMode::Permissioned,
    )
    .await
    .unwrap();
    let paths = projection.visible_paths();
    assert_eq!(projection.view_key, ProjectionViewKey::Private);
    assert!(paths.iter().any(|path| path == "/README.md"));
    assert!(paths.iter().any(|path| path == "/secret.txt"));
}

#[tokio::test]
async fn permissioned_git_projection_rejects_scope_session_without_target_repo_membership() {
    let state = test_state_with_repo();
    cache_test_jwks(&state);
    let clerk_id = "user_other_owner";
    let error = git_projection_for_request(
        &state,
        &auth_headers(bearer_header_for(clerk_id, "other@example.com")),
        TEST_REPO_OWNER,
        TEST_REPO_NAME,
        GitRemoteMode::Permissioned,
    )
    .await
    .unwrap_err();
    assert_eq!(error.status(), StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn public_git_projection_ignores_credentials_and_omits_private_files() {
    let state = test_state_with_repo();
    cache_test_jwks(&state);
    repo_with_secret(&state, "/owner-secret.txt").await;
    let projection = git_projection_for_request(
        &state,
        &auth_headers(bearer_header()),
        TEST_REPO_OWNER,
        TEST_REPO_NAME,
        GitRemoteMode::Public,
    )
    .await
    .unwrap();
    let paths = projection.visible_paths();
    assert_eq!(projection.view_key, ProjectionViewKey::Public);
    assert!(paths.iter().any(|path| path == "/README.md"));
    assert!(!paths.iter().any(|path| path == "/owner-secret.txt"));
}
