use super::*;

#[test]
fn clerk_token_verifies_issuer_signature_expiration_and_subject() {
    let jwt = token_with_audience(TEST_CLERK_USER_ID, serde_json::json!(TEST_CLERK_AUDIENCE));
    let identity =
        verify_clerk_token(&jwt, &test_jwks(), TEST_CLERK_ISSUER, &test_clerk_policy()).unwrap();

    assert_eq!(identity.user_id, TEST_CLERK_USER_ID);
    assert_eq!(identity.email.as_deref(), Some(TEST_OWNER_EMAIL));
    assert!(identity.email_verified);
}

#[test]
fn clerk_token_rejects_wrong_issuer() {
    let jwt = token_with_audience(TEST_CLERK_USER_ID, serde_json::json!(TEST_CLERK_AUDIENCE));
    let error = verify_clerk_token(
        &jwt,
        &test_jwks(),
        "https://other.example",
        &test_clerk_policy(),
    )
    .unwrap_err();

    assert_eq!(error.status, StatusCode::UNAUTHORIZED);
}

#[test]
fn clerk_token_requires_issuer_and_subject_claims() {
    let jwt = token_without_required_claims();
    let error = verify_clerk_token(&jwt, &test_jwks(), TEST_CLERK_ISSUER, &test_clerk_policy())
        .unwrap_err();

    assert_eq!(error.status, StatusCode::UNAUTHORIZED);
}

#[test]
fn clerk_token_requires_subject() {
    let jwt = token_with_audience("", serde_json::json!(TEST_CLERK_AUDIENCE));
    let error = verify_clerk_token(&jwt, &test_jwks(), TEST_CLERK_ISSUER, &test_clerk_policy())
        .unwrap_err();

    assert_eq!(error.status, StatusCode::UNAUTHORIZED);
}

#[test]
fn clerk_token_rejects_wrong_authorized_party() {
    let jwt = token_for_claims(
        TEST_CLERK_USER_ID,
        Some(TEST_OWNER_EMAIL.to_string()),
        true,
        Some("https://evil.example"),
        Some(serde_json::json!(TEST_CLERK_AUDIENCE)),
    );
    let error = verify_clerk_token(&jwt, &test_jwks(), TEST_CLERK_ISSUER, &test_clerk_policy())
        .unwrap_err();

    assert_eq!(error.status, StatusCode::UNAUTHORIZED);
}

#[test]
fn clerk_token_without_authorized_party_requires_matching_audience() {
    let jwt = token_with_authorized_party(TEST_CLERK_USER_ID, None);
    let error = verify_clerk_token(&jwt, &test_jwks(), TEST_CLERK_ISSUER, &test_clerk_policy())
        .unwrap_err();

    assert_eq!(error.status, StatusCode::UNAUTHORIZED);

    let policy = ClerkTokenPolicy {
        authorized_parties: vec![LOCAL_APP_ORIGIN.to_string()],
        audiences: vec![TEST_CLERK_AUDIENCE.to_string()],
    };
    let jwt = token_for_claims(
        TEST_CLERK_USER_ID,
        Some(TEST_OWNER_EMAIL.to_string()),
        true,
        None,
        Some(serde_json::json!(TEST_CLERK_AUDIENCE)),
    );
    let identity = verify_clerk_token(&jwt, &test_jwks(), TEST_CLERK_ISSUER, &policy).unwrap();

    assert_eq!(identity.user_id, TEST_CLERK_USER_ID);
}

#[test]
fn clerk_token_rejects_authorized_party_when_policy_is_missing() {
    let jwt = token(TEST_CLERK_USER_ID, true);
    let error = verify_clerk_token(
        &jwt,
        &test_jwks(),
        TEST_CLERK_ISSUER,
        &ClerkTokenPolicy {
            authorized_parties: Vec::new(),
            audiences: Vec::new(),
        },
    )
    .unwrap_err();

    assert_eq!(error.status, StatusCode::SERVICE_UNAVAILABLE);
}

#[test]
fn clerk_default_policy_requires_scope_api_audience() {
    let generic_session = token(TEST_CLERK_USER_ID, true);
    let error = verify_clerk_token(
        &generic_session,
        &test_jwks(),
        TEST_CLERK_ISSUER,
        &ClerkTokenPolicy::default(),
    )
    .unwrap_err();

    assert_eq!(error.status, StatusCode::UNAUTHORIZED);

    let api_token = token_with_audience(TEST_CLERK_USER_ID, serde_json::json!(TEST_CLERK_AUDIENCE));
    let identity = verify_clerk_token(
        &api_token,
        &test_jwks(),
        TEST_CLERK_ISSUER,
        &ClerkTokenPolicy::default(),
    )
    .unwrap();

    assert_eq!(identity.user_id, TEST_CLERK_USER_ID);
}

#[test]
fn clerk_token_audience_is_checked_when_configured() {
    let policy = ClerkTokenPolicy {
        authorized_parties: vec![LOCAL_APP_ORIGIN.to_string()],
        audiences: vec![TEST_CLERK_AUDIENCE.to_string()],
    };
    let accepted = token_with_audience(
        TEST_CLERK_USER_ID,
        serde_json::json!(["other-audience", TEST_CLERK_AUDIENCE]),
    );
    let rejected = token_with_audience(TEST_CLERK_USER_ID, serde_json::json!("other-audience"));

    verify_clerk_token(&accepted, &test_jwks(), TEST_CLERK_ISSUER, &policy).unwrap();
    let error =
        verify_clerk_token(&rejected, &test_jwks(), TEST_CLERK_ISSUER, &policy).unwrap_err();

    assert_eq!(error.status, StatusCode::UNAUTHORIZED);
}

#[test]
fn clerk_subject_resolves_to_internal_scope_user_id() {
    let state = AppState::test_state();
    let identity = ClerkIdentity {
        user_id: "user_123".to_string(),
        email: Some("Owner@Example.com".to_string()),
        email_verified: true,
    };

    let user = state.metadata.resolve_clerk_user(&identity).unwrap();

    assert!(user.id.starts_with("scope_usr_"));
    assert_ne!(user.id, "user_123");
    assert_eq!(user.handle, "owner");
    assert_eq!(user.email, TEST_OWNER_EMAIL);

    let again = state.metadata.resolve_clerk_user(&identity).unwrap();
    assert_eq!(again.id, user.id);
}

#[test]
fn clerk_user_ids_merge_by_verified_email() {
    let state = AppState::test_state();
    let first_identity = ClerkIdentity {
        user_id: "user_first".to_string(),
        email: Some(TEST_OWNER_EMAIL.to_string()),
        email_verified: true,
    };
    let second_identity = ClerkIdentity {
        user_id: "user_second".to_string(),
        email: Some(TEST_OWNER_EMAIL.to_string()),
        email_verified: true,
    };

    let first = state.metadata.resolve_clerk_user(&first_identity).unwrap();
    let second = state.metadata.resolve_clerk_user(&second_identity).unwrap();

    assert_eq!(first.id, second.id);
    assert_eq!(second.email, TEST_OWNER_EMAIL);
    let catalog = state.metadata.test_catalog().unwrap();
    assert_eq!(catalog.users.len(), 1);
    assert!(catalog.users.contains_key(&first.id));
}

#[test]
fn clerk_user_requires_verified_email() {
    let state = AppState::test_state();
    let identity = ClerkIdentity {
        user_id: "user_unverified".to_string(),
        email: Some(TEST_OWNER_EMAIL.to_string()),
        email_verified: false,
    };

    let error = state.metadata.resolve_clerk_user(&identity).unwrap_err();

    assert_eq!(error.status, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn known_clerk_session_uses_current_identity_without_persisting_snapshot() {
    let state = test_state_with_jwks();
    let identity = ClerkIdentity {
        user_id: TEST_CLERK_USER_ID.to_string(),
        email: Some(TEST_OWNER_EMAIL.to_string()),
        email_verified: true,
    };
    state.metadata.resolve_clerk_user(&identity).unwrap();

    let response = router(state.clone())
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/v1/session")
                .header(
                    AUTHORIZATION,
                    bearer_header_for(TEST_CLERK_USER_ID, "renamed@example.com"),
                )
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = response_json(response).await;
    assert_eq!(body["identity"]["email"], "renamed@example.com");

    let catalog = state.metadata.test_catalog().unwrap();
    let user = catalog.users.get(&test_owner_id()).unwrap();
    assert_eq!(user.email, TEST_OWNER_EMAIL);
}

#[tokio::test]
async fn missing_clerk_identity_still_bootstraps_from_session_read() {
    let state = test_state_with_jwks();
    let response = router(state.clone())
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/v1/session")
                .header(AUTHORIZATION, bearer_header())
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = response_json(response).await;
    assert_eq!(body["identity"]["user_id"], test_owner_id());

    let catalog = state.metadata.test_catalog().unwrap();
    let user = catalog.users.get(&test_owner_id()).unwrap();
    assert_eq!(user.email, TEST_OWNER_EMAIL);
}

#[tokio::test]
async fn clerk_verifier_requires_configured_issuer() {
    let verifier = ClerkVerifier::new_with_policy(
        None,
        Some("http://127.0.0.1/.well-known/jwks.json".to_string()),
        test_clerk_policy(),
    );
    let jwt = token(TEST_CLERK_USER_ID, true);
    let error = verifier.verify(&jwt).await.unwrap_err();

    assert_eq!(error.status, StatusCode::SERVICE_UNAVAILABLE);
}
