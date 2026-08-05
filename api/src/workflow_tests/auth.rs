use super::*;
use axum::{Json, Router, extract::State, response::IntoResponse, routing::get};
use std::sync::{
    Mutex,
    atomic::{AtomicUsize, Ordering},
};
use tokio::{net::TcpListener, task::JoinHandle, time::Duration};

#[derive(Clone)]
struct MockJwksState {
    response: Arc<Mutex<MockJwksResponse>>,
    requests: Arc<AtomicUsize>,
}

struct MockJwksResponse {
    keys: Option<JwkSet>,
    delay: Duration,
}

struct MockJwksServer {
    url: String,
    state: MockJwksState,
    task: JoinHandle<()>,
}

impl MockJwksServer {
    async fn start(keys: JwkSet) -> Self {
        let state = MockJwksState {
            response: Arc::new(Mutex::new(MockJwksResponse {
                keys: Some(keys),
                delay: Duration::ZERO,
            })),
            requests: Arc::new(AtomicUsize::new(0)),
        };
        let app = Router::new()
            .route("/jwks", get(mock_jwks))
            .with_state(state.clone());
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let task = tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        Self {
            url: format!("http://{address}/jwks"),
            state,
            task,
        }
    }

    fn respond_with(&self, keys: Option<JwkSet>) {
        self.state.response.lock().unwrap().keys = keys;
    }

    fn set_delay(&self, delay: Duration) {
        self.state.response.lock().unwrap().delay = delay;
    }

    fn request_count(&self) -> usize {
        self.state.requests.load(Ordering::SeqCst)
    }
}

impl Drop for MockJwksServer {
    fn drop(&mut self) {
        self.task.abort();
    }
}

async fn mock_jwks(State(state): State<MockJwksState>) -> axum::response::Response {
    state.requests.fetch_add(1, Ordering::SeqCst);
    let (keys, delay) = {
        let response = state.response.lock().unwrap();
        (response.keys.clone(), response.delay)
    };
    tokio::time::sleep(delay).await;
    match keys {
        Some(keys) => Json(keys).into_response(),
        None => StatusCode::SERVICE_UNAVAILABLE.into_response(),
    }
}

fn rotated_jwks() -> JwkSet {
    let mut keys = test_jwks();
    keys.keys[0].common.key_id = Some("rotated-key".to_string());
    keys
}

fn token_signed_with(kid: &str) -> String {
    sign_claims_with_kid(
        serde_json::json!({
            "iss": TEST_CLERK_ISSUER,
            "exp": unix_now() + 300,
            "sub": TEST_CLERK_USER_ID,
            "email": TEST_OWNER_EMAIL,
            "email_verified": true,
            "azp": LOCAL_APP_ORIGIN,
            "aud": TEST_CLERK_AUDIENCE,
        }),
        kid,
    )
}

fn verifier_for(
    server: &MockJwksServer,
    fresh_for: Duration,
    stale_for: Duration,
) -> ClerkVerifier {
    verifier_with_unknown_key_cooldown(server, fresh_for, stale_for, Duration::ZERO)
}

fn verifier_with_unknown_key_cooldown(
    server: &MockJwksServer,
    fresh_for: Duration,
    stale_for: Duration,
    unknown_key_refresh_cooldown: Duration,
) -> ClerkVerifier {
    ClerkVerifier::new_with_cache_timing(
        Some(TEST_CLERK_ISSUER.to_string()),
        Some(server.url.clone()),
        test_clerk_policy(),
        fresh_for,
        stale_for,
        unknown_key_refresh_cooldown,
    )
}

#[tokio::test]
async fn clerk_token_verifies_issuer_signature_expiration_and_subject() {
    let jwt = token_with_audience(TEST_CLERK_USER_ID, serde_json::json!(TEST_CLERK_AUDIENCE));
    let identity =
        verify_clerk_token(&jwt, &test_jwks(), TEST_CLERK_ISSUER, &test_clerk_policy()).unwrap();

    assert_eq!(identity.subject, TEST_CLERK_USER_ID);
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
        assert_eq!(error.kind, crate::error::ErrorKind::Unauthorized);
    }
}

#[test]
fn clerk_token_policy_cases() {
    use crate::error::ErrorKind::{ServiceUnavailable, Unauthorized};
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
                .subject,
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

    assert_eq!(error.kind, crate::error::ErrorKind::ServiceUnavailable);
}

#[tokio::test]
async fn clerk_verifier_refreshes_once_for_a_rotated_signing_key() {
    let server = MockJwksServer::start(test_jwks()).await;
    let verifier = verifier_for(&server, Duration::from_secs(60), Duration::from_secs(60));

    verifier
        .verify(&token_signed_with("test-key"))
        .await
        .unwrap();
    server.respond_with(Some(rotated_jwks()));
    let identity = verifier
        .verify(&token_signed_with("rotated-key"))
        .await
        .unwrap();

    assert_eq!(identity.subject, TEST_CLERK_USER_ID);
    assert_eq!(server.request_count(), 2);
}

#[tokio::test]
async fn clerk_verifier_stops_trusting_a_removed_key_after_freshness_expires() {
    let server = MockJwksServer::start(test_jwks()).await;
    let verifier = verifier_for(&server, Duration::ZERO, Duration::from_secs(60));
    let old_token = token_signed_with("test-key");

    verifier.verify(&old_token).await.unwrap();
    server.respond_with(Some(rotated_jwks()));
    let error = verifier.verify(&old_token).await.unwrap_err();

    assert_eq!(error.kind, crate::error::ErrorKind::Unauthorized);
    assert_eq!(server.request_count(), 2);
}

#[tokio::test]
async fn clerk_verifier_uses_last_known_good_keys_only_within_the_outage_grace() {
    let server = MockJwksServer::start(test_jwks()).await;
    let token = token_signed_with("test-key");
    let verifier = verifier_for(&server, Duration::ZERO, Duration::from_secs(60));
    verifier.verify(&token).await.unwrap();
    server.respond_with(None);

    verifier.verify(&token).await.unwrap();
    assert_eq!(server.request_count(), 2);

    let expired_server = MockJwksServer::start(test_jwks()).await;
    let expired = verifier_for(&expired_server, Duration::ZERO, Duration::ZERO);
    expired.verify(&token).await.unwrap();
    expired_server.respond_with(None);
    let error = expired.verify(&token).await.unwrap_err();

    assert_eq!(error.kind, crate::error::ErrorKind::ServiceUnavailable);
    assert_eq!(expired_server.request_count(), 2);
}

#[tokio::test]
async fn clerk_verifier_reports_unknown_key_outage_as_infrastructure_failure() {
    let server = MockJwksServer::start(test_jwks()).await;
    let verifier = verifier_for(&server, Duration::from_secs(60), Duration::from_secs(60));
    verifier
        .verify(&token_signed_with("test-key"))
        .await
        .unwrap();
    server.respond_with(None);

    let error = verifier
        .verify(&token_signed_with("rotated-key"))
        .await
        .unwrap_err();

    assert_eq!(error.kind, crate::error::ErrorKind::ServiceUnavailable);
    assert_eq!(server.request_count(), 2);
}

#[tokio::test]
async fn concurrent_unknown_key_requests_share_one_refresh() {
    let server = MockJwksServer::start(test_jwks()).await;
    let verifier = verifier_for(&server, Duration::from_secs(60), Duration::from_secs(60));
    verifier
        .verify(&token_signed_with("test-key"))
        .await
        .unwrap();
    server.respond_with(Some(rotated_jwks()));
    server.set_delay(Duration::from_millis(40));

    let rotated_token = token_signed_with("rotated-key");
    let mut requests = Vec::new();
    for _ in 0..16 {
        let verifier = verifier.clone();
        let token = rotated_token.clone();
        requests.push(tokio::spawn(async move { verifier.verify(&token).await }));
    }
    for request in requests {
        assert_eq!(request.await.unwrap().unwrap().subject, TEST_CLERK_USER_ID);
    }

    assert_eq!(server.request_count(), 2);
}

#[tokio::test]
async fn sequential_unknown_keys_wait_for_the_successful_refresh_cooldown() {
    let server = MockJwksServer::start(test_jwks()).await;
    let cooldown = Duration::from_millis(500);
    let verifier = verifier_with_unknown_key_cooldown(
        &server,
        Duration::from_secs(60),
        Duration::from_secs(60),
        cooldown,
    );
    verifier
        .verify(&token_signed_with("test-key"))
        .await
        .unwrap();

    tokio::time::sleep(cooldown + Duration::from_millis(50)).await;
    let first_miss = verifier
        .verify(&token_signed_with("arbitrary-key-one"))
        .await
        .unwrap_err();
    let second_miss = verifier
        .verify(&token_signed_with("arbitrary-key-two"))
        .await
        .unwrap_err();
    assert_eq!(first_miss.kind, crate::error::ErrorKind::Unauthorized);
    assert_eq!(second_miss.kind, crate::error::ErrorKind::Unauthorized);
    assert_eq!(server.request_count(), 2);

    server.respond_with(Some(rotated_jwks()));
    let cooldown_miss = verifier
        .verify(&token_signed_with("rotated-key"))
        .await
        .unwrap_err();
    assert_eq!(cooldown_miss.kind, crate::error::ErrorKind::Unauthorized);
    assert_eq!(server.request_count(), 2);

    tokio::time::sleep(cooldown + Duration::from_millis(50)).await;
    verifier
        .verify(&token_signed_with("rotated-key"))
        .await
        .unwrap();
    assert_eq!(server.request_count(), 3);
}
