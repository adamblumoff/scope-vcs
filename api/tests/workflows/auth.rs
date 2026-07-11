use super::*;

#[tokio::test]
async fn clerk_token_verifies_issuer_signature_expiration_and_subject() {
    let jwt = token_with_audience(TEST_CLERK_USER_ID, serde_json::json!(TEST_CLERK_AUDIENCE));
    let identity =
        verify_clerk_token(&jwt, &test_jwks(), TEST_CLERK_ISSUER, &test_clerk_policy()).unwrap();

    assert_eq!(identity.user_id, TEST_CLERK_USER_ID);
    assert_eq!(identity.email.as_deref(), Some(TEST_OWNER_EMAIL));
    assert!(identity.email_verified);
}

#[test]
fn clerk_token_rejects_invalid_identity_and_origin_claims() {
    let cases = [
        (
            token_with_audience(TEST_CLERK_USER_ID, serde_json::json!(TEST_CLERK_AUDIENCE)),
            "https://other.example",
        ),
        (token_without_required_claims(), TEST_CLERK_ISSUER),
        (
            token_with_audience("", serde_json::json!(TEST_CLERK_AUDIENCE)),
            TEST_CLERK_ISSUER,
        ),
        (
            token_for_claims(
                TEST_CLERK_USER_ID,
                Some(TEST_OWNER_EMAIL.to_string()),
                true,
                Some("https://evil.example"),
                Some(serde_json::json!(TEST_CLERK_AUDIENCE)),
            ),
            TEST_CLERK_ISSUER,
        ),
    ];
    for (jwt, issuer) in cases {
        let error =
            verify_clerk_token(&jwt, &test_jwks(), issuer, &test_clerk_policy()).unwrap_err();
        assert_eq!(error.kind, scope_core::error::ErrorKind::Unauthorized);
    }
}

#[test]
fn clerk_token_policy_cases() {
    use scope_core::error::ErrorKind::{ServiceUnavailable, Unauthorized};
    for (token, policy, kind) in [
        (
            token(TEST_CLERK_USER_ID, true),
            ClerkTokenPolicy {
                authorized_parties: vec![],
                audiences: vec![],
            },
            ServiceUnavailable,
        ),
        (
            token(TEST_CLERK_USER_ID, true),
            ClerkTokenPolicy::default(),
            Unauthorized,
        ),
        (
            token_with_audience(TEST_CLERK_USER_ID, serde_json::json!("other")),
            test_clerk_policy(),
            Unauthorized,
        ),
    ] {
        assert_eq!(
            verify_clerk_token(&token, &test_jwks(), TEST_CLERK_ISSUER, &policy)
                .unwrap_err()
                .kind,
            kind
        );
    }
    for (token, policy) in [
        (
            token_with_audience(TEST_CLERK_USER_ID, serde_json::json!(TEST_CLERK_AUDIENCE)),
            ClerkTokenPolicy::default(),
        ),
        (
            token_with_audience(
                TEST_CLERK_USER_ID,
                serde_json::json!(["other", TEST_CLERK_AUDIENCE]),
            ),
            test_clerk_policy(),
        ),
    ] {
        assert_eq!(
            verify_clerk_token(&token, &test_jwks(), TEST_CLERK_ISSUER, &policy)
                .unwrap()
                .user_id,
            TEST_CLERK_USER_ID
        );
    }
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

    assert_eq!(error.kind, scope_core::error::ErrorKind::ServiceUnavailable);
}
