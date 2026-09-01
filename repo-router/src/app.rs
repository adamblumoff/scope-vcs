use crate::{
    BackendDiscovery, RouterConfig, backend_selection::BackendSelector,
    discovery::DiscoveryFreshness, proxy,
};
use axum::{Router, routing::get};
use std::sync::Arc;

pub(crate) struct RouterState {
    pub(crate) discovery: BackendDiscovery,
    pub(crate) http: reqwest::Client,
    pub(crate) selector: BackendSelector,
    pub(crate) upload_pack_replay_max_bytes: usize,
}

pub fn router(config: RouterConfig) -> anyhow::Result<Router> {
    let discovery = BackendDiscovery::new(&config);
    let http = reqwest::Client::builder()
        .connect_timeout(config.connect_timeout)
        .read_timeout(config.read_timeout)
        .build()?;
    let selector = BackendSelector::new(config.read_replicas);
    Ok(router_with_state(RouterState {
        discovery,
        http,
        selector,
        upload_pack_replay_max_bytes: config.upload_pack_replay_max_bytes,
    }))
}

fn router_with_state(state: RouterState) -> Router {
    Router::new()
        .route("/healthz", get(health))
        .route("/readyz", get(ready))
        .fallback(proxy::repository_request)
        .with_state(Arc::new(state))
}

async fn health() -> &'static str {
    "ok"
}

async fn ready(
    axum::extract::State(state): axum::extract::State<Arc<RouterState>>,
) -> Result<&'static str, axum::http::StatusCode> {
    match state.discovery.backends().await {
        Ok(discovery) if discovery.freshness == DiscoveryFreshness::Fresh => Ok("ready"),
        Ok(discovery) => {
            tracing::warn!(
                backend_count = discovery.backends.len(),
                snapshot_age_ms = discovery.age.as_millis(),
                discovery_state = "stale",
                "Git router readiness is using bounded stale discovery"
            );
            Ok("degraded")
        }
        Err(error) => {
            tracing::warn!(%error, "Git router backend discovery is not ready");
            Err(axum::http::StatusCode::SERVICE_UNAVAILABLE)
        }
    }
}

#[cfg(test)]
pub(crate) fn test_router(backends: Vec<std::net::SocketAddr>) -> Router {
    test_router_with_read_replicas(backends, 1)
}

#[cfg(test)]
pub(crate) fn test_router_with_read_replicas(
    backends: Vec<std::net::SocketAddr>,
    read_replicas: usize,
) -> Router {
    router_with_state(RouterState {
        discovery: BackendDiscovery::fixed(backends),
        http: reqwest::Client::new(),
        selector: BackendSelector::new(read_replicas),
        upload_pack_replay_max_bytes: 64 * 1024 * 1024,
    })
}

#[cfg(test)]
pub(crate) fn test_router_with_state(
    discovery: BackendDiscovery,
    http: reqwest::Client,
    read_replicas: usize,
    upload_pack_replay_max_bytes: usize,
) -> Router {
    router_with_state(RouterState {
        discovery,
        http,
        selector: BackendSelector::new(read_replicas),
        upload_pack_replay_max_bytes,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{body::Body, http::Request};
    use std::time::Duration;
    use tower::ServiceExt;

    async fn readiness(discovery: BackendDiscovery) -> (axum::http::StatusCode, String) {
        let response = test_router_with_state(discovery, reqwest::Client::new(), 1, 1024)
            .oneshot(Request::get("/readyz").body(Body::empty()).unwrap())
            .await
            .unwrap();
        let status = response.status();
        let body = axum::body::to_bytes(response.into_body(), 1024)
            .await
            .unwrap();
        (status, String::from_utf8(body.to_vec()).unwrap())
    }

    #[tokio::test]
    async fn bounded_stale_discovery_is_visibly_degraded() {
        let discovery = BackendDiscovery::fixed_with_age(
            vec!["127.0.0.1:8080".parse().unwrap()],
            Duration::from_secs(10),
            Duration::from_secs(30),
            Duration::from_secs(11),
            Some("transient DNS failure"),
        );

        let (status, body) = readiness(discovery).await;

        assert_eq!(status, axum::http::StatusCode::OK);
        assert_eq!(body, "degraded");
    }

    #[tokio::test]
    async fn expired_discovery_fails_readiness() {
        let discovery = BackendDiscovery::fixed_with_age(
            vec!["127.0.0.1:8080".parse().unwrap()],
            Duration::from_secs(10),
            Duration::from_secs(30),
            Duration::from_secs(31),
            Some("DNS unavailable"),
        );

        let (status, _) = readiness(discovery).await;

        assert_eq!(status, axum::http::StatusCode::SERVICE_UNAVAILABLE);
    }
}
