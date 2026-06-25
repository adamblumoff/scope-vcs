use super::*;

#[test]
fn clerk_token_verifies_issuer_signature_expiration_and_subject() {
    let jwt = token(TEST_CLERK_USER_ID, true);
    let identity = verify_clerk_token(&jwt, &test_jwks(), TEST_CLERK_ISSUER).unwrap();

    assert_eq!(identity.user_id, TEST_CLERK_USER_ID);
    assert_eq!(identity.email.as_deref(), Some(TEST_OWNER_EMAIL));
    assert!(identity.email_verified);
}

#[test]
fn clerk_token_rejects_wrong_issuer() {
    let jwt = token(TEST_CLERK_USER_ID, true);
    let error = verify_clerk_token(&jwt, &test_jwks(), "https://other.example").unwrap_err();

    assert_eq!(error.status, StatusCode::UNAUTHORIZED);
}

#[test]
fn clerk_token_requires_issuer_and_subject_claims() {
    let jwt = token_without_required_claims();
    let error = verify_clerk_token(&jwt, &test_jwks(), TEST_CLERK_ISSUER).unwrap_err();

    assert_eq!(error.status, StatusCode::UNAUTHORIZED);
}

#[test]
fn clerk_token_requires_subject() {
    let jwt = token("", true);
    let error = verify_clerk_token(&jwt, &test_jwks(), TEST_CLERK_ISSUER).unwrap_err();

    assert_eq!(error.status, StatusCode::UNAUTHORIZED);
}

#[test]
fn clerk_user_id_is_the_local_account_id() {
    let state = AppState::test_state();
    let identity = ClerkIdentity {
        user_id: "user_123".to_string(),
        email: Some("Owner@Example.com".to_string()),
        email_verified: true,
    };

    let user = ensure_user_for_identity(&state, &identity).unwrap();

    assert_eq!(user.id, "user_123");
    assert_eq!(user.handle, "owner");
    assert_eq!(user.email, TEST_OWNER_EMAIL);
}

#[test]
fn clerk_user_ids_do_not_merge_by_verified_email() {
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

    let first = ensure_user_for_identity(&state, &first_identity).unwrap();
    let second = ensure_user_for_identity(&state, &second_identity).unwrap();

    assert_ne!(first.id, second.id);
    let catalog = state.metadata.test_catalog().unwrap();
    assert_eq!(catalog.users.len(), 2);
    assert!(catalog.users.contains_key("user_first"));
    assert!(catalog.users.contains_key("user_second"));
}

#[tokio::test]
async fn clerk_verifier_requires_configured_issuer() {
    let verifier = ClerkVerifier::new(
        None,
        Some("http://127.0.0.1/.well-known/jwks.json".to_string()),
    );
    let jwt = token(TEST_CLERK_USER_ID, true);
    let error = verifier.verify(&jwt).await.unwrap_err();

    assert_eq!(error.status, StatusCode::SERVICE_UNAVAILABLE);
}
