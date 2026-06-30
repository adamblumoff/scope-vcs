use super::*;
use axum::http::header::{
    ACCESS_CONTROL_ALLOW_HEADERS, ACCESS_CONTROL_ALLOW_METHODS, ACCESS_CONTROL_ALLOW_ORIGIN,
    ACCESS_CONTROL_REQUEST_HEADERS, ACCESS_CONTROL_REQUEST_METHOD, ORIGIN,
};

#[tokio::test]
async fn cors_preflight_names_authorization_header_explicitly() {
    let response = router(test_state_with_repo())
        .oneshot(
            Request::builder()
                .method("OPTIONS")
                .uri("/v1/repos/owner/repo/events")
                .header(ORIGIN, LOCAL_APP_ORIGIN)
                .header(ACCESS_CONTROL_REQUEST_METHOD, "GET")
                .header(ACCESS_CONTROL_REQUEST_HEADERS, "authorization")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response.headers().get(ACCESS_CONTROL_ALLOW_ORIGIN).unwrap(),
        "*"
    );

    let allow_headers = response
        .headers()
        .get(ACCESS_CONTROL_ALLOW_HEADERS)
        .unwrap()
        .to_str()
        .unwrap()
        .to_ascii_lowercase();
    assert_ne!(allow_headers, "*");
    assert!(allow_headers.contains("authorization"));

    let allow_methods = response
        .headers()
        .get(ACCESS_CONTROL_ALLOW_METHODS)
        .unwrap()
        .to_str()
        .unwrap();
    assert_ne!(allow_methods, "*");
    assert!(allow_methods.contains("GET"));
}
