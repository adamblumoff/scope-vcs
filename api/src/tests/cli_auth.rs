use super::*;

#[tokio::test]
async fn cli_browser_login_exchanges_local_callback_for_cli_token() {
    let app = router(test_state_with_jwks());
    let start = app
        .clone()
        .oneshot(start_browser_login_request(
            "http://127.0.0.1:49152/scope-cli-callback",
        ))
        .await
        .unwrap();
    assert_eq!(start.status(), StatusCode::OK);
    let start = response_json(start).await;
    let request_id = start["request_id"].as_str().unwrap();
    let request_secret = start["request_secret"].as_str().unwrap();
    assert!(request_id.starts_with(CLI_BROWSER_LOGIN_ID_PREFIX));
    assert!(request_secret.starts_with(CLI_BROWSER_LOGIN_SECRET_PREFIX));
    let authorization_url = start["authorization_url"].as_str().unwrap();
    assert!(authorization_url.starts_with(&format!("{LOCAL_APP_ORIGIN}/cli-login?")));
    assert!(authorization_url.contains(request_id));
    assert!(!authorization_url.contains(request_secret));

    let complete = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/v1/cli/browser-login/{request_id}/complete"))
                .header(AUTHORIZATION, bearer_header())
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(complete.status(), StatusCode::OK);
    let complete = response_json(complete).await;
    let callback_url = reqwest::Url::parse(complete["callback_url"].as_str().unwrap()).unwrap();
    assert_eq!(callback_url.scheme(), "http");
    assert_eq!(callback_url.host_str(), Some("127.0.0.1"));
    assert_eq!(callback_url.port(), Some(49152));
    let query = callback_url
        .query_pairs()
        .into_owned()
        .collect::<BTreeMap<_, _>>();
    assert_eq!(query.get("request_id").unwrap(), request_id);
    let callback_code = query.get("code").unwrap();
    assert!(callback_code.starts_with(CLI_CALLBACK_CODE_PREFIX));

    let exchanged = app
        .clone()
        .oneshot(exchange_browser_login_request(
            request_id,
            request_secret,
            callback_code,
        ))
        .await
        .unwrap();
    assert_eq!(exchanged.status(), StatusCode::OK);
    let exchanged = response_json(exchanged).await;
    let cli_token = exchanged["session_token"].as_str().unwrap();
    assert!(cli_token.starts_with(CLI_SESSION_TOKEN_PREFIX));
    assert_eq!(exchanged["identity"]["user_id"], test_owner_id());

    let consumed = app
        .clone()
        .oneshot(exchange_browser_login_request(
            request_id,
            request_secret,
            callback_code,
        ))
        .await
        .unwrap();
    assert_eq!(consumed.status(), StatusCode::CONFLICT);

    let session = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/v1/session")
                .header(AUTHORIZATION, format!("Bearer {cli_token}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(session.status(), StatusCode::OK);
}

#[tokio::test]
async fn cli_browser_login_rejects_non_loopback_callbacks() {
    let app = router(test_state_with_jwks());

    let non_loopback = app
        .clone()
        .oneshot(start_browser_login_request("https://scopevcs.com/callback"))
        .await
        .unwrap();
    assert_eq!(non_loopback.status(), StatusCode::BAD_REQUEST);

    let wrong_path = app
        .clone()
        .oneshot(start_browser_login_request("http://127.0.0.1:49152/other"))
        .await
        .unwrap();
    assert_eq!(wrong_path.status(), StatusCode::BAD_REQUEST);

    let existing_query = app
        .oneshot(start_browser_login_request(
            "http://127.0.0.1:49152/scope-cli-callback?next=/other",
        ))
        .await
        .unwrap();
    assert_eq!(existing_query.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn cli_browser_login_start_is_rate_limited() {
    let app = router(test_state_with_jwks());

    for _ in 0..crate::auth::device::MAX_BROWSER_LOGIN_STARTS_PER_WINDOW {
        let response = app
            .clone()
            .oneshot(start_browser_login_request(
                "http://127.0.0.1:49152/scope-cli-callback",
            ))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
    }

    let response = app
        .oneshot(start_browser_login_request(
            "http://127.0.0.1:49152/scope-cli-callback",
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::TOO_MANY_REQUESTS);
}

#[tokio::test]
async fn cli_exchange_grant_is_single_use_and_sessions_are_revocable() {
    let app = router(test_state_with_jwks());

    let grant = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/cli/exchange-grants")
                .header(AUTHORIZATION, bearer_header())
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(grant.status(), StatusCode::OK);
    let grant = response_json(grant).await;
    let exchange_token = grant["exchange_token"].as_str().unwrap();
    assert!(exchange_token.starts_with(CLI_EXCHANGE_GRANT_PREFIX));

    let exchanged = app
        .clone()
        .oneshot(exchange_grant_request(exchange_token))
        .await
        .unwrap();
    assert_eq!(exchanged.status(), StatusCode::OK);
    let exchanged = response_json(exchanged).await;
    let cli_token = exchanged["session_token"].as_str().unwrap();
    assert!(cli_token.starts_with(CLI_SESSION_TOKEN_PREFIX));

    let reused = app
        .clone()
        .oneshot(exchange_grant_request(exchange_token))
        .await
        .unwrap();
    assert_eq!(reused.status(), StatusCode::CONFLICT);

    let sessions = app
        .clone()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/v1/cli/sessions")
                .header(AUTHORIZATION, bearer_header())
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(sessions.status(), StatusCode::OK);
    let sessions = response_json(sessions).await;
    let session_id = sessions["sessions"][0]["id"].as_str().unwrap();
    assert!(session_id.starts_with(CLI_SESSION_ID_PREFIX));

    let revoked = app
        .clone()
        .oneshot(
            Request::builder()
                .method("DELETE")
                .uri(format!("/v1/cli/sessions/{session_id}"))
                .header(AUTHORIZATION, bearer_header())
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(revoked.status(), StatusCode::NO_CONTENT);

    let session = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/v1/session")
                .header(AUTHORIZATION, format!("Bearer {cli_token}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(session.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn cli_session_read_does_not_update_last_used() {
    let app = router(test_state_with_jwks());

    let grant = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/cli/exchange-grants")
                .header(AUTHORIZATION, bearer_header())
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(grant.status(), StatusCode::OK);
    let grant = response_json(grant).await;
    let exchange_token = grant["exchange_token"].as_str().unwrap();

    let exchanged = app
        .clone()
        .oneshot(exchange_grant_request(exchange_token))
        .await
        .unwrap();
    assert_eq!(exchanged.status(), StatusCode::OK);
    let cli_token = response_json(exchanged).await["session_token"]
        .as_str()
        .unwrap()
        .to_string();

    let session = app
        .clone()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/v1/session")
                .header(AUTHORIZATION, format!("Bearer {cli_token}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(session.status(), StatusCode::OK);

    let sessions = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/v1/cli/sessions")
                .header(AUTHORIZATION, bearer_header())
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(sessions.status(), StatusCode::OK);
    let sessions = response_json(sessions).await;
    assert_eq!(
        sessions["sessions"][0]["last_used_at_unix"],
        serde_json::Value::Null
    );
}

#[tokio::test]
async fn list_cli_sessions_does_not_refresh_clerk_user_snapshot() {
    let state = test_state_with_jwks();
    let identity = ClerkIdentity {
        user_id: TEST_CLERK_USER_ID.to_string(),
        email: Some(TEST_OWNER_EMAIL.to_string()),
        email_verified: true,
    };
    state.metadata.resolve_clerk_user(&identity).await.unwrap();

    let response = router(state.clone())
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/v1/cli/sessions")
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
    let catalog = state.metadata.test_catalog().unwrap();
    let user = catalog.users.get(&test_owner_id()).unwrap();
    assert_eq!(user.email, TEST_OWNER_EMAIL);
}

#[tokio::test]
async fn cli_exchange_grant_reconciles_clerk_snapshot_before_minting_session() {
    let state = test_state_with_jwks();
    let identity = ClerkIdentity {
        user_id: TEST_CLERK_USER_ID.to_string(),
        email: Some(TEST_OWNER_EMAIL.to_string()),
        email_verified: true,
    };
    state.metadata.resolve_clerk_user(&identity).await.unwrap();
    let app = router(state.clone());

    let grant = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/cli/exchange-grants")
                .header(
                    AUTHORIZATION,
                    bearer_header_for(TEST_CLERK_USER_ID, "renamed@example.com"),
                )
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(grant.status(), StatusCode::OK);
    let exchange_token = response_json(grant).await["exchange_token"]
        .as_str()
        .unwrap()
        .to_string();

    let exchanged = app
        .clone()
        .oneshot(exchange_grant_request(&exchange_token))
        .await
        .unwrap();
    assert_eq!(exchanged.status(), StatusCode::OK);
    let exchanged = response_json(exchanged).await;
    assert_eq!(exchanged["identity"]["email"], "renamed@example.com");
    let cli_token = exchanged["session_token"].as_str().unwrap();

    let session = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/v1/session")
                .header(AUTHORIZATION, format!("Bearer {cli_token}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(session.status(), StatusCode::OK);
    let session = response_json(session).await;
    assert_eq!(session["identity"]["email"], "renamed@example.com");

    let catalog = state.metadata.test_catalog().unwrap();
    let user = catalog.users.get(&test_owner_id()).unwrap();
    assert_eq!(user.email, "renamed@example.com");
}

fn start_browser_login_request(callback_url: &str) -> Request<Body> {
    Request::builder()
        .method("POST")
        .uri("/v1/cli/browser-login")
        .header(CONTENT_TYPE, "application/json")
        .body(Body::from(
            serde_json::json!({ "callback_url": callback_url }).to_string(),
        ))
        .unwrap()
}

fn exchange_browser_login_request(
    request_id: &str,
    request_secret: &str,
    callback_code: &str,
) -> Request<Body> {
    Request::builder()
        .method("POST")
        .uri(format!("/v1/cli/browser-login/{request_id}/exchange"))
        .header(CONTENT_TYPE, "application/json")
        .body(Body::from(
            serde_json::json!({
                "request_secret": request_secret,
                "callback_code": callback_code,
            })
            .to_string(),
        ))
        .unwrap()
}

fn exchange_grant_request(exchange_token: &str) -> Request<Body> {
    Request::builder()
        .method("POST")
        .uri("/v1/cli/exchange-grants/exchange")
        .header(CONTENT_TYPE, "application/json")
        .body(Body::from(
            serde_json::json!({ "exchange_token": exchange_token }).to_string(),
        ))
        .unwrap()
}
