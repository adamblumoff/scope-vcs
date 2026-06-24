use super::*;

#[tokio::test]
async fn setup_route_is_owner_only_and_hides_stored_first_push_secret() {
    let state = test_state_with_repo();
    cache_test_jwks(&state);
    {
        let mut catalog = lock_catalog(&state).unwrap();
        let repo = catalog.repositories.get_mut(TEST_REPO_ID).unwrap();
        let (_, token) = generate_first_push_token(&test_owner_id()).unwrap();
        repo.record.publication_state = RepoPublicationState::PendingFirstPush;
        repo.first_push_token = Some(token);
    };
    let app = router(state);

    let public_response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/v1/repos/owner/repo/setup")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(public_response.status(), StatusCode::UNAUTHORIZED);

    let non_owner_response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/v1/repos/owner/repo/setup")
                .header(
                    AUTHORIZATION,
                    bearer_header_for("pairwise-stranger", "stranger@example.com"),
                )
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(non_owner_response.status(), StatusCode::NOT_FOUND);

    let non_owner_regenerate_response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/repos/owner/repo/setup-token")
                .header(
                    AUTHORIZATION,
                    bearer_header_for("pairwise-stranger", "stranger@example.com"),
                )
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(
        non_owner_regenerate_response.status(),
        StatusCode::NOT_FOUND
    );

    let owner_response = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/v1/repos/owner/repo/setup")
                .header(AUTHORIZATION, bearer_header())
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(owner_response.status(), StatusCode::OK);
    let body = response_json(owner_response).await;
    assert_eq!(body["repo"]["id"], TEST_REPO_ID);
    assert_eq!(body["token"]["status"], "Active");
    assert!(body["token"]["secret"].is_null());
}

#[tokio::test]
async fn setup_token_regeneration_rotates_first_push_and_git_push_tokens() {
    let state = test_state_with_repo();
    cache_test_jwks(&state);
    let (old_hash, old_push_hash) = {
        let mut catalog = lock_catalog(&state).unwrap();
        let repo = catalog.repositories.get_mut(TEST_REPO_ID).unwrap();
        let (_, token) = generate_first_push_token(&test_owner_id()).unwrap();
        let (_, push_token) = generate_git_push_token(&test_owner_id()).unwrap();
        let old_hash = token.token_hash.clone();
        let old_push_hash = push_token.token_hash.clone();
        repo.record.publication_state = RepoPublicationState::PendingFirstPush;
        repo.first_push_token = Some(token);
        repo.git_push_token = Some(push_token);
        (old_hash, old_push_hash)
    };

    let response = router(state.clone())
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/repos/owner/repo/setup-token")
                .header(AUTHORIZATION, bearer_header())
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let body = response_json(response).await;
    let secret = body["token"]["secret"].as_str().unwrap();
    assert!(secret.starts_with("scope_fp_"));
    let push_secret = body["push_token"]["secret"].as_str().unwrap();
    assert!(push_secret.starts_with("scope_git_"));
    let catalog = lock_catalog(&state).unwrap();
    let repo = catalog.repositories.get(TEST_REPO_ID).unwrap();
    let new_hash = &repo.first_push_token.as_ref().unwrap().token_hash;
    let new_push_hash = &repo.git_push_token.as_ref().unwrap().token_hash;
    assert_ne!(new_hash, &old_hash);
    assert_ne!(new_hash, secret);
    assert!(repo.first_push_token.as_ref().unwrap().secret.is_none());
    assert_ne!(new_push_hash, &old_push_hash);
    assert_ne!(new_push_hash, push_secret);
}

#[tokio::test]
async fn git_credential_regeneration_is_owner_only_and_recreates_git_push_token() {
    let state = test_state_with_repo();
    cache_test_jwks(&state);
    {
        let mut catalog = lock_catalog(&state).unwrap();
        let repo = catalog.repositories.get_mut(TEST_REPO_ID).unwrap();
        repo.record.publication_state = RepoPublicationState::Published;
        repo.git_push_token = None;
    }
    let app = router(state.clone());

    let public_response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/repos/owner/repo/git-credential")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(public_response.status(), StatusCode::UNAUTHORIZED);

    let non_owner_response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/repos/owner/repo/git-credential")
                .header(
                    AUTHORIZATION,
                    bearer_header_for("pairwise-stranger", "stranger@example.com"),
                )
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(non_owner_response.status(), StatusCode::FORBIDDEN);

    let owner_response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/repos/owner/repo/git-credential")
                .header(AUTHORIZATION, bearer_header())
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(owner_response.status(), StatusCode::OK);
    let body = response_json(owner_response).await;
    assert_eq!(body["git_remote_path"], "/git/owner/repo");
    assert_eq!(body["remote_name"], "scope");
    let secret = body["push_token"]["secret"].as_str().unwrap();
    assert!(secret.starts_with("scope_git_"));
    let catalog = lock_catalog(&state).unwrap();
    let repo = catalog.repositories.get(TEST_REPO_ID).unwrap();
    let new_hash = &repo.git_push_token.as_ref().unwrap().token_hash;
    assert_ne!(new_hash, secret);
}

#[tokio::test]
async fn clone_credential_creation_requires_repo_membership() {
    let state = test_state_with_repo();
    cache_test_jwks(&state);
    let member_pairwise_sub = "pairwise-member";
    let member_email = "member@example.com";
    let member_id = identity_user_id(&ShooIdentity {
        pairwise_sub: member_pairwise_sub.to_string(),
        email: Some(member_email.to_string()),
        email_verified: true,
    });
    {
        let mut catalog = lock_catalog(&state).unwrap();
        catalog.users.insert(
            member_id.clone(),
            UserAccount {
                id: member_id.clone(),
                handle: "member".to_string(),
                email: member_email.to_string(),
                email_verified: true,
                access: AccountAccess::Member,
            },
        );
        let repo = catalog.repositories.get_mut(TEST_REPO_ID).unwrap();
        repo.memberships.push(RepoMembership {
            repo_id: TEST_REPO_ID.to_string(),
            user_id: member_id.clone(),
            role: RepoRole::Reader,
        });
    }
    let app = router(state.clone());

    let public_response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/repos/owner/repo/clone-credential")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(public_response.status(), StatusCode::UNAUTHORIZED);

    let stranger_response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/repos/owner/repo/clone-credential")
                .header(
                    AUTHORIZATION,
                    bearer_header_for("pairwise-stranger", "stranger@example.com"),
                )
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(stranger_response.status(), StatusCode::FORBIDDEN);

    let member_response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/repos/owner/repo/clone-credential")
                .header(
                    AUTHORIZATION,
                    bearer_header_for(member_pairwise_sub, member_email),
                )
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(member_response.status(), StatusCode::OK);
    let body = response_json(member_response).await;
    assert_eq!(body["git_remote_path"], "/git/owner/repo");
    let secret = body["token"]["secret"].as_str().unwrap();
    assert!(secret.starts_with("scope_clone_"));
    let catalog = lock_catalog(&state).unwrap();
    let repo = catalog.repositories.get(TEST_REPO_ID).unwrap();
    assert_eq!(repo.git_clone_tokens.len(), 1);
    assert_eq!(repo.git_clone_tokens[0].user_id, member_id);
    assert_ne!(repo.git_clone_tokens[0].token_hash, secret);
    let first_hash = repo.git_clone_tokens[0].token_hash.clone();
    drop(catalog);

    let second_response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/repos/owner/repo/clone-credential")
                .header(
                    AUTHORIZATION,
                    bearer_header_for(member_pairwise_sub, member_email),
                )
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(second_response.status(), StatusCode::OK);
    let second_body = response_json(second_response).await;
    let second_secret = second_body["token"]["secret"].as_str().unwrap();
    assert!(second_secret.starts_with("scope_clone_"));
    assert_ne!(second_secret, secret);
    let catalog = lock_catalog(&state).unwrap();
    let repo = catalog.repositories.get(TEST_REPO_ID).unwrap();
    assert_eq!(repo.git_clone_tokens.len(), 2);
    assert!(
        repo.git_clone_tokens
            .iter()
            .any(|token| token.token_hash == first_hash)
    );
}

#[tokio::test]
async fn clone_credential_creation_requires_published_repo() {
    let state = test_state_with_repo();
    cache_test_jwks(&state);
    {
        let mut catalog = lock_catalog(&state).unwrap();
        let repo = catalog.repositories.get_mut(TEST_REPO_ID).unwrap();
        repo.record.publication_state = RepoPublicationState::PendingFirstPush;
    }

    let response = router(state)
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/repos/owner/repo/clone-credential")
                .header(AUTHORIZATION, bearer_header())
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::CONFLICT);
}

#[test]
fn first_push_token_response_uses_persisted_expiry() {
    let token = FirstPushToken {
        token_hash: "sha256:test".to_string(),
        secret: Some("scope_fp_test".to_string()),
        owner_user_id: test_owner_id(),
        created_at_unix: 1000,
        expires_at_unix: 1000 + (60 * 60 * 24),
        used_at_unix: None,
    };

    let active = first_push_token_response(&token, 1000, None);
    assert_eq!(active.status, FirstPushTokenStatus::Active);
    assert_eq!(active.expires_at_unix, token.expires_at_unix);
    assert_eq!(active.secret.as_deref(), None);

    let minted = first_push_token_response(&token, 1000, Some("scope_fp_new".to_string()));
    assert_eq!(minted.status, FirstPushTokenStatus::Active);
    assert_eq!(minted.secret.as_deref(), Some("scope_fp_new"));

    let expired = first_push_token_response(&token, token.expires_at_unix, None);
    assert_eq!(expired.status, FirstPushTokenStatus::Expired);
    assert!(expired.secret.is_none());
}
