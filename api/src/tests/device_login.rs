use super::*;

#[tokio::test]
async fn cli_device_login_exchanges_browser_auth_for_cli_token() {
    let state = test_state_with_jwks();
    let app = router(state.clone());

    let start = app
        .clone()
        .oneshot(start_device_login_request())
        .await
        .unwrap();
    assert_eq!(start.status(), StatusCode::OK);
    let start = response_json(start).await;
    let device_code = start["device_code"].as_str().unwrap();
    let user_code = start["user_code"].as_str().unwrap();
    assert!(device_code.starts_with(CLI_DEVICE_CODE_PREFIX));
    assert_eq!(user_code.len(), 16);
    assert_eq!(
        start["verification_url"].as_str().unwrap(),
        format!("{LOCAL_APP_ORIGIN}/cli-login")
    );

    let pending = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/v1/cli/device-login/{device_code}/poll"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(pending.status(), StatusCode::OK);
    let pending = response_json(pending).await;
    assert_eq!(pending["status"], "Pending");
    assert!(pending["session_token"].is_null());

    let complete = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/v1/cli/device-login/{user_code}/complete"))
                .header(AUTHORIZATION, bearer_header())
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(complete.status(), StatusCode::OK);

    let authorized = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/v1/cli/device-login/{device_code}/poll"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(authorized.status(), StatusCode::OK);
    let authorized = response_json(authorized).await;
    assert_eq!(authorized["status"], "Complete");
    assert_eq!(authorized["identity"]["user_id"], test_owner_id());
    let cli_token = authorized["session_token"].as_str().unwrap();
    assert!(cli_token.starts_with(CLI_SESSION_TOKEN_PREFIX));

    let consumed = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/v1/cli/device-login/{device_code}/poll"))
                .body(Body::empty())
                .unwrap(),
        )
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
    let session = response_json(session).await;
    assert_eq!(session["identity"]["user_id"], test_owner_id());
    assert_eq!(session["user"]["handle"], "owner");
}

#[tokio::test]
async fn cli_device_login_completion_requires_clerk_auth() {
    let app = router(test_state_with_jwks());
    let start = app
        .clone()
        .oneshot(start_device_login_request())
        .await
        .unwrap();
    let user_code = response_json(start).await["user_code"]
        .as_str()
        .unwrap()
        .to_string();

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/v1/cli/device-login/{user_code}/complete"))
                .header(
                    AUTHORIZATION,
                    format!("Bearer {CLI_SESSION_TOKEN_PREFIX}nope"),
                )
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn cli_session_can_be_revoked() {
    let app = router(test_state_with_jwks());
    let start = app
        .clone()
        .oneshot(start_device_login_request())
        .await
        .unwrap();
    let start = response_json(start).await;
    let device_code = start["device_code"].as_str().unwrap();
    let user_code = start["user_code"].as_str().unwrap();

    let complete = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/v1/cli/device-login/{user_code}/complete"))
                .header(AUTHORIZATION, bearer_header())
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(complete.status(), StatusCode::OK);

    let authorized = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/v1/cli/device-login/{device_code}/poll"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(authorized.status(), StatusCode::OK);
    let cli_token = response_json(authorized).await["session_token"]
        .as_str()
        .unwrap()
        .to_string();

    let revoke = app
        .clone()
        .oneshot(
            Request::builder()
                .method("DELETE")
                .uri("/v1/cli/session")
                .header(AUTHORIZATION, format!("Bearer {cli_token}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(revoke.status(), StatusCode::NO_CONTENT);

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
async fn cli_device_login_start_is_rate_limited() {
    let app = router(test_state_with_jwks());

    for _ in 0..crate::auth::device::MAX_DEVICE_LOGIN_STARTS_PER_WINDOW {
        let response = app
            .clone()
            .oneshot(start_device_login_request())
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
    }

    let response = app.oneshot(start_device_login_request()).await.unwrap();
    assert_eq!(response.status(), StatusCode::TOO_MANY_REQUESTS);
}

#[tokio::test]
async fn cli_device_login_completion_is_single_use() {
    let app = router(test_state_with_jwks());
    let start = app
        .clone()
        .oneshot(start_device_login_request())
        .await
        .unwrap();
    let user_code = response_json(start).await["user_code"]
        .as_str()
        .unwrap()
        .to_string();

    let first = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/v1/cli/device-login/{user_code}/complete"))
                .header(AUTHORIZATION, bearer_header())
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(first.status(), StatusCode::OK);

    let second = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/v1/cli/device-login/{user_code}/complete"))
                .header(AUTHORIZATION, bearer_header())
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(second.status(), StatusCode::CONFLICT);
}

fn start_device_login_request() -> Request<Body> {
    Request::builder()
        .method("POST")
        .uri("/v1/cli/device-login")
        .body(Body::empty())
        .unwrap()
}
