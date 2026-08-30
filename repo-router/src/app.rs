use crate::{BackendDiscovery, RouterConfig, backend_selection::BackendSelector, proxy};
use axum::{Router, routing::get};
use std::sync::Arc;

pub(crate) struct RouterState {
    pub(crate) discovery: BackendDiscovery,
    pub(crate) http: reqwest::Client,
    pub(crate) selector: BackendSelector,
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
    state
        .discovery
        .backends()
        .await
        .map(|_| "ready")
        .map_err(|error| {
            tracing::warn!(%error, "Git router backend discovery is not ready");
            axum::http::StatusCode::SERVICE_UNAVAILABLE
        })
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
    })
}
