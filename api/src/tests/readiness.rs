use super::*;

#[tokio::test]
async fn healthz_stays_cheap_and_readyz_checks_dependencies() {
    let app = router(AppState::test_state());

    let health = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/healthz")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(health.status(), StatusCode::OK);
    assert_eq!(
        response_json(health).await,
        serde_json::json!({
            "status": "ok",
            "service": "api",
        })
    );

    let ready = app
        .oneshot(
            Request::builder()
                .uri("/readyz")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
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
}

#[tokio::test]
async fn readyz_reports_unavailable_without_leaking_dependency_errors() {
    let mut state = AppState::test_state();
    state.object_store = Arc::new(UnavailableObjectStore);

    let response = router(state)
        .oneshot(
            Request::builder()
                .uri("/readyz")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);

    let json = response_json(response).await;
    assert_eq!(
        json,
        serde_json::json!({
            "status": "unavailable",
            "service": "api",
            "checks": [
                {"name": "database", "status": "ok"},
                {"name": "object_store", "status": "unavailable"}
            ]
        })
    );
    assert!(!json.to_string().contains("secret"));
    assert!(!json.to_string().contains("internal"));
}

struct UnavailableObjectStore;

impl crate::object_store::ObjectStore for UnavailableObjectStore {
    fn put(&self, _key: &str, _bytes: &[u8]) -> Result<(), scope_core::error::ApiError> {
        Ok(())
    }

    fn get(&self, _key: &str) -> Result<Vec<u8>, scope_core::error::ApiError> {
        Ok(Vec::new())
    }

    fn delete(&self, _key: &str) -> Result<(), scope_core::error::ApiError> {
        Ok(())
    }

    fn readiness_check(&self) -> Result<(), scope_core::error::ApiError> {
        Err(scope_core::error::ApiError::service_unavailable(
            "secret internal object-store hostname is unavailable",
        ))
    }
}
