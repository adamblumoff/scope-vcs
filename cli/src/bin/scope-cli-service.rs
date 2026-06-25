use anyhow::Context;
use axum::{
    Router,
    body::Body,
    extract::{Path, State},
    http::{HeaderMap, HeaderValue, StatusCode, header},
    response::{IntoResponse, Response},
    routing::get,
};
use scope_cli::{
    distribution::DistributionManifest,
    installers::{posix_install_script, windows_install_script},
};
use std::{env, net::SocketAddr, path::PathBuf, sync::Arc};

#[derive(Clone)]
struct AppState {
    artifact_dir: Arc<PathBuf>,
    manifest: &'static DistributionManifest,
    public_url: Option<Arc<str>>,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let port = env::var("PORT")
        .ok()
        .and_then(|value| value.parse::<u16>().ok())
        .unwrap_or(8080);
    let state = AppState {
        artifact_dir: Arc::new(
            env::var("SCOPE_CLI_ARTIFACT_DIR")
                .map(PathBuf::from)
                .unwrap_or_else(|_| PathBuf::from("./dist")),
        ),
        manifest: DistributionManifest::bundled(),
        public_url: env::var("SCOPE_CLI_PUBLIC_URL").ok().map(Arc::from),
    };
    let app = Router::new()
        .route("/healthz", get(healthz))
        .route("/readyz", get(readyz))
        .route("/install.sh", get(install))
        .route("/install.ps1", get(install_windows))
        .route("/downloads/{artifact}", get(download))
        .with_state(state);
    let addr = SocketAddr::from(([0, 0, 0, 0], port));
    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .with_context(|| format!("bind {addr}"))?;

    axum::serve(listener, app)
        .await
        .context("serve CLI service")
}

async fn healthz() -> impl IntoResponse {
    (
        [("content-type", "application/json")],
        r#"{"status":"ok","service":"cli"}"#,
    )
}

async fn readyz(State(state): State<AppState>) -> Response {
    let missing = missing_downloads(&state);
    if missing.is_empty() {
        (
            StatusCode::OK,
            [("content-type", "application/json")],
            r#"{"status":"ok","service":"cli"}"#,
        )
            .into_response()
    } else {
        let body = format!(
            r#"{{"status":"unavailable","service":"cli","missing":{}}}"#,
            serde_json::to_string(&missing).unwrap_or_else(|_| "[]".to_string())
        );
        (
            StatusCode::SERVICE_UNAVAILABLE,
            [("content-type", "application/json")],
            body,
        )
            .into_response()
    }
}

async fn install(State(state): State<AppState>, headers: HeaderMap) -> impl IntoResponse {
    (
        [("content-type", "text/x-shellscript; charset=utf-8")],
        posix_install_script(&public_url(&state, &headers), state.manifest),
    )
}

async fn install_windows(State(state): State<AppState>, headers: HeaderMap) -> impl IntoResponse {
    (
        [("content-type", "text/plain; charset=utf-8")],
        windows_install_script(&public_url(&state, &headers), state.manifest),
    )
}

async fn download(State(state): State<AppState>, Path(artifact): Path<String>) -> Response {
    let Some(file_name) = state.manifest.downloadable_file(&artifact) else {
        return StatusCode::NOT_FOUND.into_response();
    };

    match tokio::fs::read(state.artifact_dir.join(file_name)).await {
        Ok(bytes) => {
            let mut response = Body::from(bytes).into_response();
            response.headers_mut().insert(
                header::CONTENT_TYPE,
                HeaderValue::from_static("application/octet-stream"),
            );
            response
        }
        Err(_) => StatusCode::NOT_FOUND.into_response(),
    }
}

fn public_url(state: &AppState, headers: &HeaderMap) -> String {
    if let Some(url) = state.public_url.as_deref() {
        return url.to_string();
    }

    let host = headers
        .get(header::HOST)
        .and_then(|value| value.to_str().ok())
        .unwrap_or("localhost:8080");
    let proto = headers
        .get("x-forwarded-proto")
        .and_then(|value| value.to_str().ok())
        .unwrap_or("https");
    format!("{proto}://{host}")
}

fn missing_downloads(state: &AppState) -> Vec<String> {
    state
        .manifest
        .required_downloads()
        .filter(|file_name| !state.artifact_dir.join(file_name).is_file())
        .collect()
}
