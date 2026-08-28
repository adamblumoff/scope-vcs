use crate::{app::RouterState, backend_selection::GitRequestKind, rank_backends, repository_key};
use axum::{
    body::Body,
    extract::{Request, State},
    http::{HeaderMap, HeaderName, StatusCode, header},
    response::{IntoResponse, Response},
};
use std::sync::Arc;

pub(crate) async fn repository_request(
    State(state): State<Arc<RouterState>>,
    request: Request,
) -> Response {
    let Some(repository) = repository_key(request.uri().path()) else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let backends = match state.discovery.backends().await {
        Ok(backends) => backends,
        Err(error) => return upstream_unavailable(&repository, error),
    };
    let identities = backends
        .iter()
        .map(|backend| backend.identity.clone())
        .collect::<Vec<_>>();
    let kind = GitRequestKind::classify(request.method(), request.uri());
    let ranked = rank_backends(&repository, &identities);
    let Some(rank) = state.selector.select_index(kind, ranked.len()) else {
        return upstream_unavailable(&repository, "no API replicas are available");
    };
    let selected = ranked[rank];
    let backend = backends
        .iter()
        .find(|backend| backend.identity == selected)
        .expect("ranked backend came from discovered backends");

    let method = request.method().clone();
    let path_and_query = request
        .uri()
        .path_and_query()
        .map(|value| value.as_str())
        .unwrap_or_else(|| request.uri().path());
    let url = format!("http://{}{path_and_query}", backend.address);
    let headers = forwarded_headers(request.headers());
    let body = reqwest::Body::wrap_stream(request.into_body().into_data_stream());

    tracing::info!(repository, backend = selected, backend_rank = rank + 1, ?kind, %method, "routing Git request");
    let upstream = match state
        .http
        .request(method, url)
        .headers(headers)
        .body(body)
        .send()
        .await
    {
        Ok(response) => response,
        Err(error) => return upstream_unavailable(&repository, error),
    };

    let status = upstream.status();
    let headers = forwarded_headers(upstream.headers());
    let mut response = Response::new(Body::from_stream(upstream.bytes_stream()));
    *response.status_mut() = status;
    *response.headers_mut() = headers;
    response
}

fn forwarded_headers(headers: &HeaderMap) -> HeaderMap {
    let connection_headers = headers
        .get(header::CONNECTION)
        .and_then(|value| value.to_str().ok())
        .map(|value| {
            value
                .split(',')
                .filter_map(|name| HeaderName::from_bytes(name.trim().as_bytes()).ok())
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    headers
        .iter()
        .filter(|(name, _)| !is_hop_by_hop(name) && !connection_headers.contains(name))
        .map(|(name, value)| (name.clone(), value.clone()))
        .collect()
}

fn is_hop_by_hop(name: &HeaderName) -> bool {
    matches!(
        name.as_str(),
        "connection"
            | "keep-alive"
            | "proxy-authenticate"
            | "proxy-authorization"
            | "te"
            | "trailer"
            | "transfer-encoding"
            | "upgrade"
            | "host"
    )
}

fn upstream_unavailable(repository: &str, error: impl std::fmt::Display) -> Response {
    tracing::warn!(repository, %error, "Git router upstream unavailable");
    (
        StatusCode::SERVICE_UNAVAILABLE,
        "Git service is temporarily unavailable",
    )
        .into_response()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        RouterConfig,
        app::{test_router, test_router_with_read_replicas},
    };
    use axum::http::Method;
    use axum::{Router, routing::any};
    use std::collections::BTreeSet;
    use tower::ServiceExt;

    async fn identified_backend() -> std::net::SocketAddr {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let identity = address.to_string();
        let upstream = Router::new().fallback(any(move || {
            let identity = identity.clone();
            async move { identity }
        }));
        tokio::spawn(async move { axum::serve(listener, upstream).await.unwrap() });
        address
    }

    async fn selected_backend(router: &Router, method: Method, uri: &str) -> String {
        let response = router
            .clone()
            .oneshot(
                Request::builder()
                    .method(method)
                    .uri(uri)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        String::from_utf8(
            axum::body::to_bytes(response.into_body(), 1024)
                .await
                .unwrap()
                .to_vec(),
        )
        .unwrap()
    }

    #[tokio::test]
    async fn streams_git_requests_and_responses_through_the_selected_backend() {
        let upstream = Router::new().fallback(any(|request: Request| async move {
            let marker = request.headers().get("x-test-marker").cloned().unwrap();
            let mut response = Response::new(request.into_body());
            response.headers_mut().insert("x-test-marker", marker);
            response
        }));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, upstream).await.unwrap() });
        let router = test_router(vec![address]);
        let request = Request::builder()
            .method("POST")
            .uri("/git/permissioned/scope/router/git-upload-pack")
            .header("x-test-marker", "preserved")
            .body(Body::from("pack request"))
            .unwrap();

        let response = router.oneshot(request).await.unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(response.headers()["x-test-marker"], "preserved");
        assert_eq!(
            axum::body::to_bytes(response.into_body(), 1024)
                .await
                .unwrap(),
            "pack request"
        );
    }

    #[tokio::test]
    async fn ends_an_upstream_read_that_stops_making_progress() {
        let upstream =
            Router::new().fallback(any(|| async { std::future::pending::<String>().await }));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, upstream).await.unwrap() });
        let router = crate::router(RouterConfig {
            backend_authority: address.to_string(),
            dns_refresh: std::time::Duration::from_secs(1),
            connect_timeout: std::time::Duration::from_secs(1),
            read_timeout: std::time::Duration::from_millis(20),
            read_replicas: 1,
        })
        .unwrap();

        let response = router
            .oneshot(
                Request::builder()
                    .uri("/git/public/scope/router/info/refs?service=git-upload-pack")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    }

    #[tokio::test]
    async fn refuses_non_git_paths() {
        let response = test_router(Vec::new())
            .oneshot(
                Request::builder()
                    .uri("/v1/repos/scope/router")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn spreads_upload_pack_reads_across_the_ranked_prefix() {
        let first = identified_backend().await;
        let second = identified_backend().await;
        let third = identified_backend().await;
        let addresses = vec![first, second, third];
        let identities = addresses
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>();
        let expected = rank_backends("scope/router", &identities)
            .into_iter()
            .take(2)
            .map(str::to_owned)
            .collect::<BTreeSet<_>>();
        let router = test_router_with_read_replicas(addresses, 2);

        let actual = [
            (
                Method::GET,
                "/git/public/scope/router/info/refs?service=git-upload-pack",
            ),
            (Method::POST, "/git/public/scope/router/git-upload-pack"),
            (
                Method::GET,
                "/git/permissioned/scope/router/info/refs?service=git-upload-pack",
            ),
            (
                Method::POST,
                "/git/permissioned/scope/router/git-upload-pack",
            ),
        ];
        let mut selected = BTreeSet::new();
        for (method, uri) in actual {
            selected.insert(selected_backend(&router, method, uri).await);
        }

        assert_eq!(selected, expected);
    }

    #[tokio::test]
    async fn pins_receive_pack_operations_to_rendezvous_rank_one() {
        let first = identified_backend().await;
        let second = identified_backend().await;
        let third = identified_backend().await;
        let addresses = vec![first, second, third];
        let identities = addresses
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>();
        let expected = rank_backends("scope/router", &identities)[0];
        let router = test_router_with_read_replicas(addresses, 3);

        for (method, uri) in [
            (
                Method::GET,
                "/git/permissioned/scope/router/info/refs?service=git-receive-pack",
            ),
            (
                Method::POST,
                "/git/permissioned/scope/router/git-receive-pack",
            ),
        ] {
            assert_eq!(selected_backend(&router, method, uri).await, expected);
        }
    }
}
