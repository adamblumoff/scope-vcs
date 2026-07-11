use super::*;

async fn start_login(app: &axum::Router) -> serde_json::Value {
    let response = send_device_request(app, start_device_login_request()).await;
    assert_eq!(response.status(), StatusCode::OK);
    response_json(response).await
}

async fn device_post(
    app: &axum::Router,
    code: &str,
    action: &str,
    auth: Option<String>,
) -> Response {
    send_device_request(
        app,
        device_request(
            "POST",
            &format!("/v1/cli/device-login/{code}/{action}"),
            auth,
        ),
    )
    .await
}

#[tokio::test]
async fn cli_device_login_exchanges_browser_auth_for_cli_token() {
    let state = test_state_with_jwks();
    let app = router(state.clone());

    let start = start_login(&app).await;
    let device_code = start["device_code"].as_str().unwrap();
    let user_code = start["user_code"].as_str().unwrap();
    assert!(device_code.starts_with(CLI_DEVICE_CODE_PREFIX));
    assert_eq!(user_code.len(), 16);
    assert_eq!(
        start["verification_url"].as_str().unwrap(),
        format!("{LOCAL_APP_ORIGIN}/cli-login")
    );

    let pending = device_post(&app, device_code, "poll", None).await;
    assert_eq!(pending.status(), StatusCode::OK);
    let pending = response_json(pending).await;
    assert_eq!(pending["status"], "Pending");
    assert!(pending["session_token"].is_null());

    let complete = device_post(&app, user_code, "complete", Some(bearer_header())).await;
    assert_eq!(complete.status(), StatusCode::OK);

    let authorized = device_post(&app, device_code, "poll", None).await;
    assert_eq!(authorized.status(), StatusCode::OK);
    let authorized = response_json(authorized).await;
    assert_eq!(authorized["status"], "Complete");
    assert_eq!(authorized["identity"]["user_id"], test_owner_id());
    let cli_token = authorized["session_token"].as_str().unwrap();
    assert!(cli_token.starts_with(CLI_SESSION_TOKEN_PREFIX));

    let consumed = device_post(&app, device_code, "poll", None).await;
    assert_eq!(consumed.status(), StatusCode::CONFLICT);

    let session = send_device_request(
        &app,
        device_request("GET", "/v1/session", Some(format!("Bearer {cli_token}"))),
    )
    .await;
    assert_eq!(session.status(), StatusCode::OK);
}

#[tokio::test]
async fn cli_device_login_completion_requires_clerk_auth_and_is_single_use() {
    let app = router(test_state_with_jwks());
    let start = start_login(&app).await;
    let user_code = start["user_code"].as_str().unwrap().to_string();

    let unauthorized = device_post(
        &app,
        &user_code,
        "complete",
        Some(format!("Bearer {CLI_SESSION_TOKEN_PREFIX}nope")),
    )
    .await;
    assert_eq!(unauthorized.status(), StatusCode::UNAUTHORIZED);

    let first = device_post(&app, &user_code, "complete", Some(bearer_header())).await;
    assert_eq!(first.status(), StatusCode::OK);
    let second = device_post(&app, &user_code, "complete", Some(bearer_header())).await;
    assert_eq!(second.status(), StatusCode::CONFLICT);
}

#[tokio::test]
async fn cli_device_login_start_is_rate_limited() {
    let app = router(test_state_with_jwks());

    for _ in 0..crate::auth::device::MAX_DEVICE_LOGIN_STARTS_PER_WINDOW {
        let response = send_device_request(&app, start_device_login_request()).await;
        assert_eq!(response.status(), StatusCode::OK);
    }

    let response = send_device_request(&app, start_device_login_request()).await;
    assert_eq!(response.status(), StatusCode::TOO_MANY_REQUESTS);
}

fn start_device_login_request() -> Request<Body> {
    device_request("POST", "/v1/cli/device-login", None)
}

fn device_request(method: &str, uri: &str, bearer: Option<String>) -> Request<Body> {
    let mut request = Request::builder().method(method).uri(uri);
    if let Some(bearer) = bearer {
        request = request.header(AUTHORIZATION, bearer);
    }
    request.body(Body::empty()).unwrap()
}

async fn send_device_request(app: &axum::Router, request: Request<Body>) -> Response {
    app.clone().oneshot(request).await.unwrap()
}
