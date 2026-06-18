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
use scope_policy::ScopePath;
use scope_projection::{Projection, project_graph};
use scope_store::{AppCatalog, StoredRepository, VerifiedEmail, app_catalog};
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, net::SocketAddr, sync::Arc};
use tower_http::{cors::CorsLayer, trace::TraceLayer};
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

const USER_EMAIL_HEADER: &str = "x-scope-user-email";
const USER_EMAIL_VERIFIED_HEADER: &str = "x-scope-user-email-verified";
const TRUSTED_IDENTITY_SECRET_HEADER: &str = "x-scope-trusted-identity-secret";
const TRUSTED_IDENTITY_SECRET_ENV: &str = "SCOPE_TRUSTED_IDENTITY_HEADER_SECRET";
const MANIFEST_SIGNING_SECRET_ENV: &str = "SCOPE_MANIFEST_SIGNING_SECRET";

#[derive(Clone)]
struct AppState {
    catalog: Arc<AppCatalog>,
    trusted_identity_secret: Option<String>,
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
        .route("/v1/repos/{owner}/{repo}/projections", get(get_projection))
        .route(
            "/v1/repos/{owner}/{repo}/git-projections",
            get(get_git_projection),
        )
        .route(
            "/v1/repos/{owner}/{repo}/push-manifests",
            post(create_manifest),
        )
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
    Path((owner, repo_name)): Path<(String, String)>,
) -> Result<Json<Projection>, ApiError> {
    let repo = find_repo(&state, &owner, &repo_name)?;
    let identity = http_identity(&state, &headers)?;
    let principal = state.catalog.principal_for_repo(repo, identity.as_ref());
    Ok(Json(project_graph(&repo.policy, &repo.graph, &principal)))
}

async fn get_git_projection(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((owner, repo_name)): Path<(String, String)>,
) -> Result<Json<VirtualGitProjection>, ApiError> {
    let repo = find_repo(&state, &owner, &repo_name)?;
    let identity = http_identity(&state, &headers)?;
    let principal = state.catalog.principal_for_repo(repo, identity.as_ref());
    let projection = project_graph(&repo.policy, &repo.graph, &principal);
    Ok(Json(build_virtual_git_projection(&projection)))
}

async fn create_manifest(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((owner, repo_name)): Path<(String, String)>,
    Json(input): Json<CreateManifestRequest>,
) -> Result<Json<CreateManifestResponse>, ApiError> {
    let repo = find_repo(&state, &owner, &repo_name)?;
    let identity = http_identity(&state, &headers)?;
    let principal = state.catalog.principal_for_repo(repo, identity.as_ref());

    for changed_path in &input.changed_paths {
        let path = ScopePath::parse(changed_path).map_err(ApiError::bad_request)?;
        if !repo.policy.can_write(&principal, &path) {
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
        repo.record.id.clone(),
        principal.id,
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

fn find_repo<'a>(
    state: &'a AppState,
    owner: &str,
    name: &str,
) -> Result<&'a StoredRepository, ApiError> {
    state
        .catalog
        .repository(owner, name)
        .ok_or_else(|| ApiError::not_found(format!("repo {owner}/{name} not found")))
}

impl AppState {
    fn from_env() -> Self {
        Self {
            catalog: Arc::new(app_catalog()),
            trusted_identity_secret: non_empty_env(TRUSTED_IDENTITY_SECRET_ENV),
            manifest_signing_secret: non_empty_env(MANIFEST_SIGNING_SECRET_ENV)
                .map(|secret| Arc::<[u8]>::from(secret.into_bytes())),
        }
    }

    #[cfg(test)]
    fn test_state(
        trusted_identity_secret: Option<&str>,
        manifest_signing_secret: Option<&str>,
    ) -> Self {
        Self {
            catalog: Arc::new(app_catalog()),
            trusted_identity_secret: trusted_identity_secret.map(ToOwned::to_owned),
            manifest_signing_secret: manifest_signing_secret
                .map(|secret| Arc::<[u8]>::from(secret.as_bytes())),
        }
    }
}

fn non_empty_env(name: &str) -> Option<String> {
    std::env::var(name).ok().filter(|value| !value.is_empty())
}

fn http_identity(state: &AppState, headers: &HeaderMap) -> Result<Option<VerifiedEmail>, ApiError> {
    let Some(email) = headers
        .get(USER_EMAIL_HEADER)
        .and_then(|value| value.to_str().ok())
    else {
        return Ok(None);
    };

    let Some(expected_secret) = state.trusted_identity_secret.as_deref() else {
        return Err(ApiError::forbidden(format!(
            "trusted identity headers require {TRUSTED_IDENTITY_SECRET_ENV}"
        )));
    };
    let actual_secret = headers
        .get(TRUSTED_IDENTITY_SECRET_HEADER)
        .and_then(|value| value.to_str().ok())
        .ok_or_else(|| ApiError::forbidden(format!("missing {TRUSTED_IDENTITY_SECRET_HEADER}")))?;
    if actual_secret != expected_secret {
        return Err(ApiError::forbidden(format!(
            "invalid {TRUSTED_IDENTITY_SECRET_HEADER}"
        )));
    }

    let verified = headers
        .get(USER_EMAIL_VERIFIED_HEADER)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| value.eq_ignore_ascii_case("true"));

    Ok(Some(VerifiedEmail::new(email, verified)))
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
    use scope_policy::{Principal, PrincipalKind};
    use scope_store::{BOOTSTRAP_OWNER_USER_ID, BOOTSTRAP_REPO_NAME, BOOTSTRAP_REPO_OWNER};

    fn headers_with_identity(email: &str, verified: bool, secret: &str) -> HeaderMap {
        let mut headers = HeaderMap::new();
        headers.insert(USER_EMAIL_HEADER, email.parse().unwrap());
        headers.insert(
            USER_EMAIL_VERIFIED_HEADER,
            verified.to_string().parse().unwrap(),
        );
        headers.insert(TRUSTED_IDENTITY_SECRET_HEADER, secret.parse().unwrap());
        headers
    }

    #[test]
    fn anonymous_request_uses_public_principal() {
        let state = AppState::test_state(None, None);
        let repo = find_repo(&state, BOOTSTRAP_REPO_OWNER, BOOTSTRAP_REPO_NAME).unwrap();
        let principal = state.catalog.principal_for_repo(repo, None);

        assert_eq!(principal, Principal::public());
    }

    #[test]
    fn verified_bootstrap_email_uses_owner_principal() {
        let state = AppState::test_state(Some("trusted"), None);
        let repo = find_repo(&state, BOOTSTRAP_REPO_OWNER, BOOTSTRAP_REPO_NAME).unwrap();
        let headers = headers_with_identity("adamblumoff@gmail.com", true, "trusted");
        let identity = http_identity(&state, &headers).unwrap();
        let principal = state.catalog.principal_for_repo(repo, identity.as_ref());

        assert_eq!(principal.id, BOOTSTRAP_OWNER_USER_ID);
        assert_eq!(principal.kind, PrincipalKind::User);
    }

    #[test]
    fn unverified_bootstrap_email_uses_public_principal() {
        let state = AppState::test_state(Some("trusted"), None);
        let repo = find_repo(&state, BOOTSTRAP_REPO_OWNER, BOOTSTRAP_REPO_NAME).unwrap();
        let headers = headers_with_identity("adamblumoff@gmail.com", false, "trusted");
        let identity = http_identity(&state, &headers).unwrap();
        let principal = state.catalog.principal_for_repo(repo, identity.as_ref());

        assert_eq!(principal, Principal::public());
    }

    #[test]
    fn claimed_identity_requires_trusted_handoff_secret() {
        let state = AppState::test_state(None, None);
        let headers = headers_with_identity("adamblumoff@gmail.com", true, "trusted");
        let error = http_identity(&state, &headers).unwrap_err();

        assert_eq!(error.status, StatusCode::FORBIDDEN);
    }

    #[test]
    fn manifest_signing_secret_is_absent_unless_explicitly_configured() {
        let state = AppState::test_state(None, None);

        assert!(state.manifest_signing_secret.is_none());
    }
}
