use crate::{BackendDiscovery, RouterConfig, proxy};
use axum::{Router, routing::get};
use std::sync::Arc;

#[derive(Clone)]
pub(crate) struct RouterState {
    pub(crate) discovery: BackendDiscovery,
    pub(crate) http: reqwest::Client,
}

pub fn router(config: RouterConfig) -> anyhow::Result<Router> {
    let discovery = BackendDiscovery::new(&config);
    let http = reqwest::Client::builder()
        .connect_timeout(config.connect_timeout)
        .build()?;
    Ok(router_with_state(RouterState { discovery, http }))
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
    router_with_state(RouterState {
        discovery: BackendDiscovery::fixed(backends),
        http: reqwest::Client::new(),
    })
}
