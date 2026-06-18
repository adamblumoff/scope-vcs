use anyhow::Context;
use axum::{
    Json, Router,
    extract::{Path, Query, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::{get, post},
};
use scope_crypto::{ManifestMixedPolicy, PushManifest, SignedPushManifest, sign_manifest};
use scope_git::{VirtualGitProjection, build_virtual_git_projection};
use scope_policy::ScopePath;
use scope_projection::{Projection, project_graph};
use scope_store::{DemoRepository, demo_repository};
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, net::SocketAddr, sync::Arc};
use tower_http::{cors::CorsLayer, trace::TraceLayer};
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

const DEV_DEVICE_SECRET: &[u8] = b"scope-dev-device-secret";

#[derive(Clone)]
struct AppState {
    demo: Arc<DemoRepository>,
}

#[derive(Debug, Serialize)]
struct HealthResponse {
    status: &'static str,
    service: &'static str,
}

#[derive(Debug, Serialize)]
struct ErrorResponse {
    error: String,
}

#[derive(Debug, Deserialize)]
struct CreateManifestRequest {
    principal_id: String,
    device_id: String,
    commit_graph_hash: String,
    changed_paths: Vec<String>,
    mixed_policy: ManifestMixedPolicy,
}

#[derive(Debug, Serialize)]
struct CreateManifestResponse {
    signed_manifest: SignedPushManifest,
}

#[derive(Debug, Deserialize)]
struct GitInfoRefsQuery {
    service: Option<String>,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::registry()
        .with(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "scope_server=info,tower_http=info".into()),
        )
        .with(tracing_subscriber::fmt::layer())
        .init();

    let port = std::env::var("PORT")
        .ok()
        .and_then(|value| value.parse::<u16>().ok())
        .unwrap_or(8080);
    let addr = SocketAddr::from(([0, 0, 0, 0], port));
    let state = AppState {
        demo: Arc::new(demo_repository()),
    };

    let app = router(state);
    tracing::info!(%addr, "starting scope server");

    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .with_context(|| format!("binding server on {addr}"))?;
    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await
        .context("serving scope server")?;

    Ok(())
}

fn router(state: AppState) -> Router {
    Router::new()
        .route("/healthz", get(healthz))
        .route(
            "/v1/repos/{repo_id}/projections/{principal_id}",
            get(get_projection),
        )
        .route(
            "/v1/repos/{repo_id}/git-projections/{principal_id}",
            get(get_git_projection),
        )
        .route("/v1/repos/{repo_id}/push-manifests", post(create_manifest))
        .route("/git/{org}/{repo}/info/refs", get(git_info_refs))
        .with_state(state)
        .layer(CorsLayer::permissive())
        .layer(TraceLayer::new_for_http())
}

async fn healthz() -> Json<HealthResponse> {
    Json(HealthResponse {
        status: "ok",
        service: "scope-server",
    })
}

async fn get_projection(
    State(state): State<AppState>,
    Path((repo_id, principal_id)): Path<(String, String)>,
) -> Result<Json<Projection>, ApiError> {
    ensure_demo_repo(&repo_id)?;
    let principal = DemoRepository::projection_principal(&principal_id);
    Ok(Json(project_graph(
        &state.demo.policy,
        &state.demo.graph,
        &principal,
    )))
}

async fn get_git_projection(
    State(state): State<AppState>,
    Path((repo_id, principal_id)): Path<(String, String)>,
) -> Result<Json<VirtualGitProjection>, ApiError> {
    ensure_demo_repo(&repo_id)?;
    let principal = DemoRepository::projection_principal(&principal_id);
    let projection = project_graph(&state.demo.policy, &state.demo.graph, &principal);
    Ok(Json(build_virtual_git_projection(&projection)))
}

async fn create_manifest(
    State(state): State<AppState>,
    Path(repo_id): Path<String>,
    Json(input): Json<CreateManifestRequest>,
) -> Result<Json<CreateManifestResponse>, ApiError> {
    ensure_demo_repo(&repo_id)?;
    let principal = DemoRepository::projection_principal(&input.principal_id);

    for changed_path in &input.changed_paths {
        let path = ScopePath::parse(changed_path).map_err(ApiError::bad_request)?;
        if !state.demo.policy.can_write(&principal, &path) {
            return Err(ApiError::forbidden(format!(
                "principal {} cannot write {}",
                principal.id, path
            )));
        }
    }

    let manifest = PushManifest::new(
        repo_id,
        input.principal_id,
        input.device_id,
        input.commit_graph_hash,
        input.changed_paths,
        input.mixed_policy,
    );
    let signed_manifest = sign_manifest(manifest, DEV_DEVICE_SECRET).map_err(ApiError::internal)?;

    Ok(Json(CreateManifestResponse { signed_manifest }))
}

async fn git_info_refs(
    Path((org, repo)): Path<(String, String)>,
    Query(query): Query<GitInfoRefsQuery>,
) -> Response {
    let mut details = BTreeMap::new();
    details.insert("org", org);
    details.insert("repo", repo);
    details.insert(
        "service",
        query.service.unwrap_or_else(|| "unspecified".to_string()),
    );

    (
        StatusCode::NOT_IMPLEMENTED,
        Json(serde_json::json!({
            "error": "custom git upload-pack/receive-pack is not implemented yet",
            "details": details,
            "next": "scope-git already builds leak-checked virtual object sets; this endpoint must serve real packfiles before Git clone is enabled"
        })),
    )
        .into_response()
}

fn ensure_demo_repo(repo_id: &str) -> Result<(), ApiError> {
    if repo_id == "scope-demo" {
        Ok(())
    } else {
        Err(ApiError::not_found(format!("repo {repo_id} not found")))
    }
}

async fn shutdown_signal() {
    let ctrl_c = async {
        tokio::signal::ctrl_c()
            .await
            .expect("failed to install Ctrl+C handler");
    };

    #[cfg(unix)]
    let terminate = async {
        tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
            .expect("failed to install signal handler")
            .recv()
            .await;
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        () = ctrl_c => {},
        () = terminate => {},
    }
}

#[derive(Debug)]
struct ApiError {
    status: StatusCode,
    message: String,
}

impl ApiError {
    fn bad_request(error: impl std::error::Error) -> Self {
        Self {
            status: StatusCode::BAD_REQUEST,
            message: error.to_string(),
        }
    }

    fn forbidden(message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::FORBIDDEN,
            message: message.into(),
        }
    }

    fn not_found(message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::NOT_FOUND,
            message: message.into(),
        }
    }

    fn internal(error: impl std::error::Error) -> Self {
        Self {
            status: StatusCode::INTERNAL_SERVER_ERROR,
            message: error.to_string(),
        }
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (
            self.status,
            Json(ErrorResponse {
                error: self.message,
            }),
        )
            .into_response()
    }
}
