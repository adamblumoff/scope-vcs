use super::*;

#[test]
fn shoo_token_verifies_issuer_audience_signature_expiration_and_pairwise_sub() {
    let jwt = token("origin:http://localhost:3000", TEST_PAIRWISE_SUB, true);
    let identity = verify_shoo_token(
        &jwt,
        &test_jwks(),
        SHOO_ISSUER,
        "origin:http://localhost:3000",
    )
    .unwrap();

    assert_eq!(identity.pairwise_sub, TEST_PAIRWISE_SUB);
    assert_eq!(identity.email.as_deref(), Some(TEST_OWNER_EMAIL));
    assert!(identity.email_verified);
}

#[test]
fn shoo_token_rejects_wrong_audience() {
    let jwt = token("origin:https://other.example", TEST_PAIRWISE_SUB, true);
    let error = verify_shoo_token(
        &jwt,
        &test_jwks(),
        SHOO_ISSUER,
        "origin:http://localhost:3000",
    )
    .unwrap_err();

    assert_eq!(error.status, StatusCode::UNAUTHORIZED);
}

#[test]
fn shoo_token_requires_issuer_and_audience_claims() {
    let jwt = token_without_origin_claims();
    let error = verify_shoo_token(
        &jwt,
        &test_jwks(),
        SHOO_ISSUER,
        "origin:http://localhost:3000",
    )
    .unwrap_err();

    assert_eq!(error.status, StatusCode::UNAUTHORIZED);
}

#[test]
fn shoo_token_requires_pairwise_sub() {
    let jwt = token("origin:http://localhost:3000", "", true);
    let error = verify_shoo_token(
        &jwt,
        &test_jwks(),
        SHOO_ISSUER,
        "origin:http://localhost:3000",
    )
    .unwrap_err();

    assert_eq!(error.status, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn shoo_verifier_requires_configured_audience() {
    let verifier = ShooVerifier::new(SHOO_ISSUER, None, "http://127.0.0.1/.well-known/jwks.json");
    let jwt = token("origin:http://localhost:3000", TEST_PAIRWISE_SUB, true);
    let error = verifier.verify(&jwt).await.unwrap_err();

    assert_eq!(error.status, StatusCode::SERVICE_UNAVAILABLE);
}
