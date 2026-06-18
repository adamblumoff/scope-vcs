use anyhow::Context;
use axum::{
    Json, Router,
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::{get, post},
};
use scope_crypto::{ManifestMixedPolicy, PushManifest, SignedPushManifest, sign_manifest};
use scope_git::{VirtualGitProjection, build_virtual_git_projection};
use scope_policy::{Principal, PrincipalKind, ScopePath};
use scope_projection::{Projection, project_graph};
use scope_store::{DemoRepository, demo_repository};
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, net::SocketAddr, sync::Arc};
use tower_http::{cors::CorsLayer, trace::TraceLayer};
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

const DEMO_AUTHORITY_HEADER: &str = "x-scope-demo-authority";
const DEMO_AUTHORITY_ENV: &str = "SCOPE_DEMO_AUTHORITY";
const MANIFEST_SIGNING_SECRET_ENV: &str = "SCOPE_MANIFEST_SIGNING_SECRET";

#[derive(Clone)]
struct AppState {
    demo: Arc<DemoRepository>,
    demo_authority_token: Option<String>,
    manifest_signing_secret: Option<Arc<[u8]>>,
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
    let state = AppState::from_env();

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
    headers: HeaderMap,
    Path((repo_id, principal_id)): Path<(String, String)>,
) -> Result<Json<Projection>, ApiError> {
    ensure_demo_repo(&repo_id)?;
    let principal = authorized_http_principal(&state, &headers, &principal_id)?;
    Ok(Json(project_graph(
        &state.demo.policy,
        &state.demo.graph,
        &principal,
    )))
}

async fn get_git_projection(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((repo_id, principal_id)): Path<(String, String)>,
) -> Result<Json<VirtualGitProjection>, ApiError> {
    ensure_demo_repo(&repo_id)?;
    let principal = authorized_http_principal(&state, &headers, &principal_id)?;
    let projection = project_graph(&state.demo.policy, &state.demo.graph, &principal);
    Ok(Json(build_virtual_git_projection(&projection)))
}

async fn create_manifest(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(repo_id): Path<String>,
    Json(input): Json<CreateManifestRequest>,
) -> Result<Json<CreateManifestResponse>, ApiError> {
    ensure_demo_repo(&repo_id)?;
    let principal = authorized_http_principal(&state, &headers, &input.principal_id)?;

    for changed_path in &input.changed_paths {
        let path = ScopePath::parse(changed_path).map_err(ApiError::bad_request)?;
        if !state.demo.policy.can_write(&principal, &path) {
            return Err(ApiError::forbidden(format!(
                "principal {} cannot write {}",
                principal.id, path
            )));
        }
    }
    let signing_secret = state.manifest_signing_secret.as_deref().ok_or_else(|| {
        ApiError::service_unavailable(format!(
            "manifest signing is disabled; set {MANIFEST_SIGNING_SECRET_ENV}"
        ))
    })?;

    let manifest = PushManifest::new(
        repo_id,
        input.principal_id,
        input.device_id,
        input.commit_graph_hash,
        input.changed_paths,
        input.mixed_policy,
    );
    let signed_manifest = sign_manifest(manifest, signing_secret).map_err(ApiError::internal)?;

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

impl AppState {
    fn from_env() -> Self {
        Self {
            demo: Arc::new(demo_repository()),
            demo_authority_token: non_empty_env(DEMO_AUTHORITY_ENV),
            manifest_signing_secret: non_empty_env(MANIFEST_SIGNING_SECRET_ENV)
                .map(|secret| Arc::<[u8]>::from(secret.into_bytes())),
        }
    }

    #[cfg(test)]
    fn test_state(
        demo_authority_token: Option<&str>,
        manifest_signing_secret: Option<&str>,
    ) -> Self {
        Self {
            demo: Arc::new(demo_repository()),
            demo_authority_token: demo_authority_token.map(ToOwned::to_owned),
            manifest_signing_secret: manifest_signing_secret
                .map(|secret| Arc::<[u8]>::from(secret.as_bytes())),
        }
    }
}

fn non_empty_env(name: &str) -> Option<String> {
    std::env::var(name).ok().filter(|value| !value.is_empty())
}

fn authorized_http_principal(
    state: &AppState,
    headers: &HeaderMap,
    requested_id: &str,
) -> Result<Principal, ApiError> {
    let principal = DemoRepository::projection_principal(requested_id);
    if principal.kind == PrincipalKind::Public {
        return Ok(principal);
    }

    ensure_demo_authority(state, headers)?;
    Ok(principal)
}

fn ensure_demo_authority(state: &AppState, headers: &HeaderMap) -> Result<(), ApiError> {
    let Some(expected) = state.demo_authority_token.as_deref() else {
        return Err(ApiError::forbidden(format!(
            "non-public demo principals require {DEMO_AUTHORITY_ENV} and {DEMO_AUTHORITY_HEADER}"
        )));
    };

    let Some(actual) = headers
        .get(DEMO_AUTHORITY_HEADER)
        .and_then(|value| value.to_str().ok())
    else {
        return Err(ApiError::forbidden(format!(
            "missing {DEMO_AUTHORITY_HEADER} for non-public demo principal"
        )));
    };

    if actual == expected {
        Ok(())
    } else {
        Err(ApiError::forbidden(format!(
            "invalid {DEMO_AUTHORITY_HEADER} for non-public demo principal"
        )))
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

    fn service_unavailable(message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::SERVICE_UNAVAILABLE,
            message: message.into(),
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

#[cfg(test)]
mod tests {
    use super::*;

    fn headers_with_demo_authority(token: &str) -> HeaderMap {
        let mut headers = HeaderMap::new();
        headers.insert(DEMO_AUTHORITY_HEADER, token.parse().unwrap());
        headers
    }

    #[test]
    fn public_principal_does_not_need_demo_authority() {
        let state = AppState::test_state(None, None);
        let principal = authorized_http_principal(&state, &HeaderMap::new(), "public").unwrap();

        assert_eq!(principal.id, "public");
        assert_eq!(principal.kind, PrincipalKind::Public);
    }

    #[test]
    fn non_public_principal_requires_configured_demo_authority() {
        let state = AppState::test_state(None, None);
        let error = authorized_http_principal(&state, &HeaderMap::new(), "team-core").unwrap_err();

        assert_eq!(error.status, StatusCode::FORBIDDEN);
    }

    #[test]
    fn non_public_principal_requires_matching_demo_authority_header() {
        let state = AppState::test_state(Some("demo-token"), None);

        assert!(authorized_http_principal(&state, &HeaderMap::new(), "team-core").is_err());
        assert!(
            authorized_http_principal(
                &state,
                &headers_with_demo_authority("wrong-token"),
                "team-core",
            )
            .is_err()
        );

        let principal = authorized_http_principal(
            &state,
            &headers_with_demo_authority("demo-token"),
            "team-core",
        )
        .unwrap();
        assert_eq!(principal.id, "team-core");
    }

    #[test]
    fn manifest_signing_secret_is_absent_unless_explicitly_configured() {
        let state = AppState::test_state(Some("demo-token"), None);

        assert!(state.manifest_signing_secret.is_none());
    }
}
