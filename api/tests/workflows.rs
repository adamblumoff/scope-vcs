use api::test_support::TestApp;
use axum::{
    body::{Body, to_bytes},
    http::{
        Request, StatusCode,
        header::{
            ACCESS_CONTROL_ALLOW_HEADERS, ACCESS_CONTROL_ALLOW_METHODS,
            ACCESS_CONTROL_ALLOW_ORIGIN, ACCESS_CONTROL_REQUEST_HEADERS,
            ACCESS_CONTROL_REQUEST_METHOD, ORIGIN,
        },
    },
};
use tower::ServiceExt;

#[tokio::test]
async fn health_and_readiness_cover_live_and_unavailable_dependencies() {
    let app = TestApp::new().router();
    let health = app
        .clone()
        .oneshot(request("GET", "/healthz"))
        .await
        .unwrap();
    assert_eq!(health.status(), StatusCode::OK);
    assert_eq!(
        response_json(health).await,
        serde_json::json!({"status": "ok", "service": "api"})
    );

    let ready = app.oneshot(request("GET", "/readyz")).await.unwrap();
    assert_eq!(ready.status(), StatusCode::OK);
    assert_eq!(
        response_json(ready).await,
        serde_json::json!({
            "status": "ok",
            "service": "api",
            "checks": [
                {"name": "database", "status": "ok"},
                {"name": "object_store", "status": "ok"}
            ]
        })
    );

    let unavailable = TestApp::new()
        .with_unavailable_object_store()
        .router()
        .oneshot(request("GET", "/readyz"))
        .await
        .unwrap();
    assert_eq!(unavailable.status(), StatusCode::SERVICE_UNAVAILABLE);
    let json = response_json(unavailable).await;
    assert_eq!(json["checks"][1]["status"], "unavailable");
    assert!(!json.to_string().contains("secret"));
    assert!(!json.to_string().contains("internal"));
}

#[tokio::test]
async fn cors_preflight_explicitly_allows_authorization_and_get() {
    let response = TestApp::new()
        .router()
        .oneshot(
            Request::builder()
                .method("OPTIONS")
                .uri("/v1/repos/owner/repo/events")
                .header(ORIGIN, "http://localhost:5173")
                .header(ACCESS_CONTROL_REQUEST_METHOD, "GET")
                .header(ACCESS_CONTROL_REQUEST_HEADERS, "authorization")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response.headers()[ACCESS_CONTROL_ALLOW_ORIGIN], "*");
    let allow_headers = response.headers()[ACCESS_CONTROL_ALLOW_HEADERS]
        .to_str()
        .unwrap()
        .to_ascii_lowercase();
    assert_ne!(allow_headers, "*");
    assert!(allow_headers.contains("authorization"));
    let allow_methods = response.headers()[ACCESS_CONTROL_ALLOW_METHODS]
        .to_str()
        .unwrap();
    assert_ne!(allow_methods, "*");
    assert!(allow_methods.contains("GET"));
}

fn request(method: &str, uri: &str) -> Request<Body> {
    Request::builder()
        .method(method)
        .uri(uri)
        .body(Body::empty())
        .unwrap()
}

async fn response_json(response: axum::response::Response) -> serde_json::Value {
    let body = to_bytes(response.into_body(), 1024 * 1024).await.unwrap();
    serde_json::from_slice(&body).unwrap()
}
