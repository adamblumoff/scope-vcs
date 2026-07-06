use super::*;

#[tokio::test]
async fn clone_credential_endpoint_accepts_cli_session_for_repo_member_projection() {
    let state = test_state_with_repo();
    cache_test_jwks(&state);
    let member_clerk_id = "user_member";
    let member_id = crate::db::scope_user_id_for_auth_identity("clerk", member_clerk_id);
    {
        let mut repo = repo_with_readme();
        repo.members.push(test_repository_member(
            TEST_REPO_ID,
            member_id.clone(),
            RepositoryMemberPermissions::default(),
        ));
        repo.policy
            .add_rule(VisibilityRule::private(
                ScopePath::parse("/member-secret.txt").unwrap(),
            ))
            .unwrap();
        repo.policy
            .add_rule(VisibilityRule::private(
                ScopePath::parse("/owner-secret.txt").unwrap(),
            ))
            .unwrap();
        repo.graph.commits[0].changes.extend([
            FileChange {
                visibility: Visibility::Private,
                path: ScopePath::parse("/member-secret.txt").unwrap(),
                old_content: None,
                new_content: Some(source_blob("member can read")),
            },
            FileChange {
                visibility: Visibility::Private,
                path: ScopePath::parse("/owner-secret.txt").unwrap(),
                old_content: None,
                new_content: Some(source_blob("owner only")),
            },
        ]);

        let mut catalog = lock_catalog(&state).unwrap();
        catalog.repositories.insert(TEST_REPO_ID.to_string(), repo);
    }
    let cli_token = cli_session_token_for_user(&state, member_clerk_id, "member@example.com").await;
    let app = router(state.clone());

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!(
                    "/v1/repos/{TEST_REPO_OWNER}/{TEST_REPO_NAME}/clone-credential"
                ))
                .header(AUTHORIZATION, format!("Bearer {cli_token}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let body = response_json(response).await;
    assert_eq!(body["git_remote_path"].as_str().unwrap(), "/git/owner/repo");
    assert_eq!(
        body["config"]["kind"].as_str().unwrap(),
        "scope.repo-config"
    );
    assert_eq!(body["config"]["visibility"]["default"], "public");
    let secret = body["token"]["secret"].as_str().unwrap();
    assert!(secret.starts_with(GIT_PUSH_TOKEN_PREFIX));
    let mut headers = HeaderMap::new();
    headers.insert(
        AUTHORIZATION,
        format!("Basic {}", BASE64.encode(format!("scope:{secret}")))
            .parse()
            .unwrap(),
    );

    let projection = git_projection_for_request(&state, &headers, TEST_REPO_OWNER, TEST_REPO_NAME)
        .await
        .unwrap();
    let visible_paths = projection
        .commits
        .iter()
        .flat_map(|commit| &commit.changes)
        .map(|change| change.path.as_str())
        .collect::<Vec<_>>();

    assert!(visible_paths.contains(&"/README.md"));
    assert!(visible_paths.contains(&"/member-secret.txt"));
    assert!(visible_paths.contains(&"/owner-secret.txt"));
}

#[tokio::test]
async fn clone_credential_endpoint_rejects_cli_session_without_target_repo_membership() {
    let state = test_state_with_repo();
    cache_test_jwks(&state);
    let other_clerk_id = "user_other_owner";
    let other_id = crate::db::scope_user_id_for_auth_identity("clerk", other_clerk_id);
    {
        let mut other_repo = test_repo(&other_id);
        other_repo.record.id = "other/owned".to_string();
        other_repo.record.owner_handle = "other".to_string();
        other_repo.record.name = "owned".to_string();
        other_repo.graph.repo_id = other_repo.record.id.clone();
        let mut catalog = lock_catalog(&state).unwrap();
        catalog
            .repositories
            .insert(other_repo.record.id.clone(), other_repo);
    }
    let cli_token = cli_session_token_for_user(&state, other_clerk_id, "other@example.com").await;

    let response = router(state)
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!(
                    "/v1/repos/{TEST_REPO_OWNER}/{TEST_REPO_NAME}/clone-credential"
                ))
                .header(AUTHORIZATION, format!("Bearer {cli_token}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn public_git_projection_without_credentials_omits_private_files() {
    let state = test_state_with_repo();
    {
        let mut repo = repo_with_readme();
        repo.policy
            .add_rule(VisibilityRule::private(
                ScopePath::parse("/owner-secret.txt").unwrap(),
            ))
            .unwrap();
        repo.graph.commits[0].changes.push(FileChange {
            visibility: Visibility::Private,
            path: ScopePath::parse("/owner-secret.txt").unwrap(),
            old_content: None,
            new_content: Some(source_blob("owner only")),
        });

        let mut catalog = lock_catalog(&state).unwrap();
        catalog.repositories.insert(TEST_REPO_ID.to_string(), repo);
    }
    let headers = HeaderMap::new();

    let projection = git_projection_for_request(&state, &headers, TEST_REPO_OWNER, TEST_REPO_NAME)
        .await
        .unwrap();
    let visible_paths = projection
        .commits
        .iter()
        .flat_map(|commit| &commit.changes)
        .map(|change| change.path.as_str())
        .collect::<Vec<_>>();

    assert!(visible_paths.contains(&"/README.md"));
    assert!(!visible_paths.contains(&"/owner-secret.txt"));
}

async fn cli_session_token_for_user(state: &AppState, user_id: &str, email: &str) -> String {
    let app = router(state.clone());
    let grant = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/cli/exchange-grants")
                .header(AUTHORIZATION, bearer_header_for(user_id, email))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(grant.status(), StatusCode::OK);
    let grant = response_json(grant).await;
    let exchange_token = grant["exchange_token"].as_str().unwrap();

    let exchanged = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/cli/exchange-grants/exchange")
                .header(CONTENT_TYPE, "application/json")
                .body(Body::from(
                    serde_json::json!({ "exchange_token": exchange_token }).to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(exchanged.status(), StatusCode::OK);
    response_json(exchanged).await["session_token"]
        .as_str()
        .unwrap()
        .to_string()
}
