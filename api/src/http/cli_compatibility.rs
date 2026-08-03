use crate::{auth::clerk::bearer_token, config::CLI_SESSION_TOKEN_PREFIX};
use axum::{
    Json,
    extract::Request,
    http::Method,
    middleware::Next,
    response::{IntoResponse, Response},
};
use scope_api_contract::{CLI_PROTOCOL_HEADER, CLI_PROTOCOL_VERSION, ErrorResponse};

pub(crate) async fn enforce(request: Request, next: Next) -> Response {
    if let Some(installed_protocol) = incompatible_cli_protocol(&request) {
        return (
            axum::http::StatusCode::UPGRADE_REQUIRED,
            Json(ErrorResponse::cli_upgrade_required(installed_protocol)),
        )
            .into_response();
    }

    next.run(request).await
}

fn incompatible_cli_protocol(request: &Request) -> Option<Option<u32>> {
    let path = request.uri().path();
    if !is_api_mutation(request.method())
        || !path.starts_with("/v1/")
        || path.starts_with("/v1/cli/")
    {
        return None;
    }
    let is_cli_session = bearer_token(request.headers())
        .ok()
        .flatten()
        .is_some_and(|token| token.starts_with(CLI_SESSION_TOKEN_PREFIX));
    if !is_cli_session {
        return None;
    }

    let installed = request
        .headers()
        .get(CLI_PROTOCOL_HEADER)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.parse::<u32>().ok());
    (installed != Some(CLI_PROTOCOL_VERSION)).then_some(installed)
}

fn is_api_mutation(method: &Method) -> bool {
    matches!(
        method,
        &Method::POST | &Method::PATCH | &Method::PUT | &Method::DELETE
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{
        Router,
        body::{Body, to_bytes},
        http::{Request as HttpRequest, StatusCode, header::AUTHORIZATION},
        middleware,
        routing::{get, post},
    };
    use tower::ServiceExt;

    fn test_router() -> Router {
        Router::new()
            .route("/v1/mutate", post(|| async { StatusCode::NO_CONTENT }))
            .route("/v1/read", get(|| async { StatusCode::NO_CONTENT }))
            .route(
                "/v1/cli/device-login/code/complete",
                post(|| async { StatusCode::NO_CONTENT }),
            )
            .route(
                "/git/private/repo/git-receive-pack",
                post(|| async { StatusCode::NO_CONTENT }),
            )
            .layer(middleware::from_fn(enforce))
    }

    fn request(method: Method, path: &str, protocol: Option<&str>) -> HttpRequest<Body> {
        let mut request = HttpRequest::builder()
            .method(method)
            .uri(path)
            .header(AUTHORIZATION, "Bearer scope_cli_fixture");
        if let Some(protocol) = protocol {
            request = request.header(CLI_PROTOCOL_HEADER, protocol);
        }
        request.body(Body::empty()).unwrap()
    }

    #[tokio::test]
    async fn incompatible_cli_mutation_returns_actionable_426() {
        let response = test_router()
            .oneshot(request(Method::POST, "/v1/mutate", Some("0")))
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::UPGRADE_REQUIRED);
        let body = to_bytes(response.into_body(), 16 * 1024).await.unwrap();
        let error: ErrorResponse = serde_json::from_slice(&body).unwrap();
        assert_eq!(error.code, "cli_upgrade_required");
        assert_eq!(error.installed_protocol, Some(0));
        assert_eq!(error.supported_protocol, Some(1));
        assert!(error.message.contains("installed Scope CLI protocol 0"));
        assert!(error.message.contains("supports protocol 1"));
        assert!(error.instruction.unwrap().contains("install.sh | sh"));
    }

    #[tokio::test]
    async fn missing_protocol_is_rejected_before_mutation() {
        let response = test_router()
            .oneshot(request(Method::POST, "/v1/mutate", None))
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::UPGRADE_REQUIRED);
        let body = to_bytes(response.into_body(), 16 * 1024).await.unwrap();
        let error: ErrorResponse = serde_json::from_slice(&body).unwrap();
        assert!(error.message.contains("protocol missing"));
        assert_eq!(error.installed_protocol, None);
        assert_eq!(error.supported_protocol, Some(1));
    }

    #[tokio::test]
    async fn future_protocol_is_rejected_until_the_api_supports_it() {
        let response = test_router()
            .oneshot(request(Method::POST, "/v1/mutate", Some("2")))
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::UPGRADE_REQUIRED);
        let body = to_bytes(response.into_body(), 16 * 1024).await.unwrap();
        let error: ErrorResponse = serde_json::from_slice(&body).unwrap();
        assert_eq!(error.installed_protocol, Some(2));
        assert_eq!(error.supported_protocol, Some(1));
        assert_eq!(
            error.instruction.as_deref(),
            Some("This Scope CLI requires API protocol 2. Retry after the Scope API is updated.")
        );
    }

    #[tokio::test]
    async fn current_cli_and_non_mutating_or_git_traffic_pass_through() {
        for request in [
            request(Method::POST, "/v1/mutate", Some("1")),
            request(Method::GET, "/v1/read", None),
            request(Method::POST, "/v1/cli/device-login/code/complete", None),
            request(Method::POST, "/git/private/repo/git-receive-pack", None),
        ] {
            let response = test_router().oneshot(request).await.unwrap();
            assert_eq!(response.status(), StatusCode::NO_CONTENT);
        }
    }

    #[tokio::test]
    async fn non_cli_bearer_mutations_are_not_gated() {
        let request = HttpRequest::builder()
            .method(Method::POST)
            .uri("/v1/mutate")
            .header(AUTHORIZATION, "Bearer clerk_fixture")
            .body(Body::empty())
            .unwrap();

        let response = test_router().oneshot(request).await.unwrap();
        assert_eq!(response.status(), StatusCode::NO_CONTENT);
    }
}
