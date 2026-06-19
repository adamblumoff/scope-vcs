use anyhow::Context;
use axum::{
    Json, Router,
    body::{Body, to_bytes},
    extract::{Path, Query, Request, State},
    http::{
        HeaderMap, HeaderValue, StatusCode,
        header::{AUTHORIZATION, CONTENT_TYPE, WWW_AUTHENTICATE},
    },
    response::{IntoResponse, Response},
    routing::{get, patch, post},
};
use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64};
use jsonwebtoken::{
    Algorithm, DecodingKey, Validation, decode, decode_header,
    jwk::{Jwk, JwkSet},
};
use scope_git::{VirtualGitProjection, build_virtual_git_projection};
use scope_policy::{Policy, Principal, ScopePath, Visibility, VisibilityRule};
use scope_projection::{
    AuthorVisibility, FileChange, LogicalCommit, MixedCommitPolicy, Projection, project_graph,
};
use scope_store::{
    AccountAccess, AppCatalog, CatalogError, FirstPushToken, FirstPushTokenStatus, PendingImport,
    PendingImportFile, RepoPublicationState, RepoRole, RepoSettings, StoredRepository, UserAccount,
    app_catalog,
};
use serde::{Deserialize, Serialize};
use sha1::Sha1;
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    fs,
    io::Write,
    net::SocketAddr,
    path::{Path as FsPath, PathBuf},
    process::{Command, Stdio},
    sync::{Arc, Mutex},
    time::Duration,
};
use tower_http::{cors::CorsLayer, trace::TraceLayer};
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

const SCOPE_APP_ORIGIN_ENV: &str = "SCOPE_APP_ORIGIN";
const SCOPE_REPO_ROOT_ENV: &str = "SCOPE_REPO_ROOT";
const SCOPE_STATE_PATH_ENV: &str = "SCOPE_STATE_PATH";
const SHOO_JWKS_URL: &str = "https://shoo.dev/.well-known/jwks.json";
const SHOO_ISSUER: &str = "https://shoo.dev";
const LOCAL_APP_ORIGIN: &str = "http://localhost:3000";
const FIRST_PUSH_TOKEN_BYTES: usize = 32;
const FIRST_PUSH_TOKEN_TTL_SECS: u64 = 60 * 60 * 24;
const EMPTY_GIT_OID: &str = "0000000000000000000000000000000000000000";
const MAX_RECEIVE_PACK_BYTES: usize = 512 * 1024 * 1024;
const MAX_PENDING_IMPORT_FILES: usize = 10_000;
const MAX_PENDING_IMPORT_BLOB_BYTES: usize = 25 * 1024 * 1024;
const MAX_PENDING_IMPORT_TOTAL_BYTES: usize = 100 * 1024 * 1024;

#[derive(Clone)]
struct AppState {
    catalog: Arc<Mutex<AppCatalog>>,
    state_path: Arc<PathBuf>,
    shoo: ShooVerifier,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
struct PersistedState {
    users: BTreeMap<String, UserAccount>,
    repositories: BTreeMap<String, StoredRepository>,
}

#[derive(Clone)]
struct ShooVerifier {
    client: reqwest::Client,
    issuer: String,
    audience: Option<String>,
    jwks_url: String,
    jwks_cache: Arc<Mutex<Option<JwkSet>>>,
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

#[derive(Debug, Serialize)]
struct AccountSessionResponse {
    identity: Option<SessionIdentity>,
    user: Option<UserResponse>,
}

#[derive(Debug, Serialize)]
struct UserResponse {
    id: String,
    handle: String,
    email: String,
    email_verified: bool,
}

impl From<UserAccount> for UserResponse {
    fn from(user: UserAccount) -> Self {
        Self {
            id: user.id,
            handle: user.handle,
            email: user.email,
            email_verified: user.email_verified,
        }
    }
}

#[derive(Debug, Serialize)]
struct SessionResponse {
    identity: Option<SessionIdentity>,
    repo: SessionRepo,
    principal_id: String,
    capabilities: SessionCapabilities,
}

#[derive(Debug, Serialize)]
struct SessionIdentity {
    pairwise_sub: String,
    email: Option<String>,
    email_verified: bool,
}

#[derive(Debug, Serialize)]
struct SessionRepo {
    id: String,
    publication_state: RepoPublicationState,
    role: Option<RepoRole>,
}

#[derive(Debug, Serialize)]
struct SessionCapabilities {
    read: bool,
    write: bool,
}

#[derive(Debug, Serialize)]
struct RepoSummaryResponse {
    id: String,
    owner_handle: String,
    name: String,
    lifecycle_state: RepoPublicationState,
    default_visibility: Visibility,
    role: RepoRole,
}

#[derive(Debug, Serialize)]
struct CreateRepoResponse {
    repo: RepoSummaryResponse,
    setup: RepoSetupResponse,
}

#[derive(Debug, Serialize)]
struct RepoSetupResponse {
    repo: RepoSummaryResponse,
    git_remote_path: String,
    remote_name: &'static str,
    push_branch: &'static str,
    push_enabled: bool,
    token: Option<FirstPushTokenResponse>,
}

#[derive(Debug, Serialize)]
struct FirstPushTokenResponse {
    status: FirstPushTokenStatus,
    created_at_unix: u64,
    expires_at_unix: u64,
    used_at_unix: Option<u64>,
    secret: Option<String>,
}

#[derive(Debug, Serialize)]
struct RepoFileResponse {
    path: String,
    oid: String,
    tracked: bool,
    visibility: Visibility,
}

#[derive(Debug, Serialize)]
struct PendingImportReviewResponse {
    publication_state: RepoPublicationState,
    default_visibility: Visibility,
    files: Vec<RepoFileResponse>,
}

#[derive(Debug, Deserialize)]
struct UpdateFileVisibilityRequest {
    path: String,
    visibility: Visibility,
}

#[derive(Debug, Deserialize)]
struct UpdateRepoSettingsRequest {
    include_ignored_files: bool,
}

#[derive(Debug, Deserialize)]
struct CreateRepoRequest {
    name: String,
    visibility: Option<Visibility>,
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
    let state = AppState::from_env()?;

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
        .route("/v1/session", get(get_account_session))
        .route("/v1/repos", get(list_repos).post(create_repo))
        .route("/v1/repos/{owner}/{repo}/setup", get(get_repo_setup))
        .route(
            "/v1/repos/{owner}/{repo}/setup-token",
            get(get_repo_setup).post(regenerate_first_push_token),
        )
        .route("/v1/repos/{owner}/{repo}/session", get(get_session))
        .route("/v1/repos/{owner}/{repo}/files", get(get_files))
        .route(
            "/v1/repos/{owner}/{repo}/pending-import",
            get(get_pending_import_review),
        )
        .route("/v1/repos/{owner}/{repo}/publish", post(publish_repo))
        .route(
            "/v1/repos/{owner}/{repo}/files/visibility",
            patch(update_file_visibility),
        )
        .route(
            "/v1/repos/{owner}/{repo}/settings",
            get(get_settings).patch(update_settings),
        )
        .route("/v1/repos/{owner}/{repo}/projections", get(get_projection))
        .route(
            "/v1/repos/{owner}/{repo}/git-projections",
            get(get_git_projection),
        )
        .route("/git/{org}/{repo}/info/refs", get(git_info_refs))
        .route("/git/{org}/{repo}/git-receive-pack", post(git_receive_pack))
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

async fn get_account_session(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<AccountSessionResponse>, ApiError> {
    let identity = http_identity(&state, &headers).await?;
    let user = match identity.as_ref() {
        Some(identity) => Some(UserResponse::from(ensure_user_for_identity(
            &state, identity,
        )?)),
        None => None,
    };

    Ok(Json(AccountSessionResponse {
        identity: identity.as_ref().map(SessionIdentity::from),
        user,
    }))
}

async fn list_repos(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<Vec<RepoSummaryResponse>>, ApiError> {
    let identity = require_identity(&state, &headers).await?;
    let user = ensure_user_for_identity(&state, &identity)?;
    let catalog = lock_catalog(&state)?;
    let mut repositories = catalog
        .repositories_for_user(&user.id)
        .into_iter()
        .filter_map(|repo| repo_summary(&catalog, repo, &user.id))
        .collect::<Vec<_>>();
    repositories.sort_by(|left, right| left.id.cmp(&right.id));

    Ok(Json(repositories))
}

async fn create_repo(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(input): Json<CreateRepoRequest>,
) -> Result<Json<CreateRepoResponse>, ApiError> {
    let identity = require_identity(&state, &headers).await?;
    let user = ensure_user_for_identity(&state, &identity)?;
    let default_visibility = input.visibility.unwrap_or(Visibility::Private);

    let created = {
        let mut catalog = lock_catalog(&state)?;
        let mut staged = catalog.clone();
        let user = staged
            .users
            .get(&user.id)
            .cloned()
            .ok_or_else(|| ApiError::internal_message("signed-in user was not persisted"))?;
        let repo_id = staged
            .create_repository(&user, &input.name, default_visibility)
            .map_err(catalog_error)?
            .record
            .id
            .clone();
        let (secret, token) = generate_first_push_token(&user.id)?;
        let now = unix_now()?;
        {
            let repo = staged
                .repositories
                .get_mut(&repo_id)
                .expect("created repository must exist");
            repo.first_push_token = Some(token);
        }
        let repo = staged
            .repositories
            .get(&repo_id)
            .expect("created repository must exist");
        let summary = repo_summary(&staged, repo, &user.id).ok_or_else(|| {
            ApiError::internal_message("created repository is missing owner role")
        })?;
        let setup = repo_setup_response(&staged, repo, &user.id, now, Some(secret))?;

        persist_catalog(&state, &staged)?;
        *catalog = staged;
        CreateRepoResponse {
            repo: summary,
            setup,
        }
    };

    Ok(Json(created))
}

async fn get_repo_setup(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((owner, repo_name)): Path<(String, String)>,
) -> Result<Json<RepoSetupResponse>, ApiError> {
    let identity = require_identity(&state, &headers).await?;
    let user = ensure_user_for_identity(&state, &identity)?;
    let repo = find_repo(&state, &owner, &repo_name)?;
    ensure_owner_setup_access(&state, &repo, &user.id)?;
    let catalog = lock_catalog(&state)?;
    Ok(Json(repo_setup_response(
        &catalog,
        &repo,
        &user.id,
        unix_now()?,
        None,
    )?))
}

async fn regenerate_first_push_token(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((owner, repo_name)): Path<(String, String)>,
) -> Result<Json<RepoSetupResponse>, ApiError> {
    let identity = require_identity(&state, &headers).await?;
    let user = ensure_user_for_identity(&state, &identity)?;
    let repo_id = scope_store::repo_id(&owner, &repo_name);
    let now = unix_now()?;

    let setup = {
        let mut catalog = lock_catalog(&state)?;
        let mut staged = catalog.clone();
        let repo = staged
            .repositories
            .get(&repo_id)
            .ok_or_else(|| ApiError::not_found(format!("repo {owner}/{repo_name} not found")))?;
        ensure_owner_setup_access_in_catalog(&staged, repo, &user.id)?;

        let (secret, token) = generate_first_push_token(&user.id)?;
        {
            let repo = staged
                .repositories
                .get_mut(&repo_id)
                .expect("repo was already checked");
            repo.first_push_token = Some(token);
        }
        let repo = staged
            .repositories
            .get(&repo_id)
            .expect("repo was already checked");
        let setup = repo_setup_response(&staged, repo, &user.id, now, Some(secret))?;

        persist_catalog(&state, &staged)?;
        *catalog = staged;
        setup
    };

    Ok(Json(setup))
}

async fn get_projection(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((owner, repo_name)): Path<(String, String)>,
) -> Result<Json<Projection>, ApiError> {
    let repo = find_repo(&state, &owner, &repo_name)?;
    let identity = http_identity(&state, &headers).await?;
    let principal = principal_for_repo(&state, &repo, identity.as_ref())?;
    ensure_repo_read(&state, &repo, &principal)?;
    Ok(Json(project_graph(&repo.policy, &repo.graph, &principal)))
}

async fn get_git_projection(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((owner, repo_name)): Path<(String, String)>,
) -> Result<Json<VirtualGitProjection>, ApiError> {
    let repo = find_repo(&state, &owner, &repo_name)?;
    let identity = http_identity(&state, &headers).await?;
    let principal = principal_for_repo(&state, &repo, identity.as_ref())?;
    ensure_repo_read(&state, &repo, &principal)?;
    let projection = project_graph(&repo.policy, &repo.graph, &principal);
    Ok(Json(build_virtual_git_projection(&projection)))
}

async fn get_session(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((owner, repo_name)): Path<(String, String)>,
) -> Result<Json<SessionResponse>, ApiError> {
    let repo = find_repo(&state, &owner, &repo_name)?;
    let identity = http_identity(&state, &headers).await?;
    let principal = principal_for_repo(&state, &repo, identity.as_ref())?;
    ensure_repo_read(&state, &repo, &principal)?;
    let root = ScopePath::root();
    let role = role_for_principal(&state, &repo, &principal)?;

    Ok(Json(SessionResponse {
        identity: identity.as_ref().map(SessionIdentity::from),
        repo: SessionRepo {
            id: repo.record.id.clone(),
            publication_state: repo.record.publication_state,
            role,
        },
        capabilities: SessionCapabilities {
            read: can_read_path(&state, &repo, &principal, &root)?,
            write: can_write_path(&state, &repo, &principal, &root)?,
        },
        principal_id: principal.id,
    }))
}

async fn get_files(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((owner, repo_name)): Path<(String, String)>,
) -> Result<Json<Vec<RepoFileResponse>>, ApiError> {
    let repo = find_repo(&state, &owner, &repo_name)?;
    let identity = http_identity(&state, &headers).await?;
    let principal = principal_for_repo(&state, &repo, identity.as_ref())?;
    ensure_repo_read(&state, &repo, &principal)?;

    Ok(Json(projected_files(&repo, &principal)))
}

async fn get_pending_import_review(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((owner, repo_name)): Path<(String, String)>,
) -> Result<Json<PendingImportReviewResponse>, ApiError> {
    let repo = find_repo(&state, &owner, &repo_name)?;
    let identity = http_identity(&state, &headers).await?;
    let principal = principal_for_repo(&state, &repo, identity.as_ref())?;
    ensure_repo_read(&state, &repo, &principal)?;
    ensure_owner(&state, &repo, &principal)?;
    ensure_pending_publish(&repo)?;

    Ok(Json(PendingImportReviewResponse {
        publication_state: repo.record.publication_state,
        default_visibility: repo.record.default_visibility,
        files: pending_import_files(&repo, &principal)?,
    }))
}

async fn publish_repo(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((owner, repo_name)): Path<(String, String)>,
) -> Result<Json<SessionRepo>, ApiError> {
    let identity = http_identity(&state, &headers).await?;
    let repo_id = scope_store::repo_id(&owner, &repo_name);
    let repo = find_repo(&state, &owner, &repo_name)?;
    let principal = principal_for_repo(&state, &repo, identity.as_ref())?;
    ensure_repo_read(&state, &repo, &principal)?;
    ensure_owner(&state, &repo, &principal)?;
    ensure_pending_publish(&repo)?;

    let updated = {
        let mut catalog = lock_catalog(&state)?;
        let mut staged = catalog.clone();
        let repo = staged
            .repositories
            .get(&repo_id)
            .ok_or_else(|| ApiError::not_found(format!("repo {owner}/{repo_name} not found")))?;
        let user_id = identity
            .as_ref()
            .map(identity_user_id)
            .ok_or_else(|| ApiError::forbidden("owner role required"))?;
        let principal = principal_for_user_id(repo, &user_id);
        if staged.role_for_principal(repo, &principal) != Some(RepoRole::Owner) {
            return Err(ApiError::forbidden("owner role required"));
        }
        ensure_pending_publish(repo)?;

        {
            let repo = staged
                .repositories
                .get_mut(&repo_id)
                .expect("repo was already checked");
            promote_pending_import(repo)?;
        }

        persist_catalog(&state, &staged)?;
        let updated = staged
            .repositories
            .get(&repo_id)
            .expect("repo was already checked")
            .record
            .clone();
        *catalog = staged;
        updated
    };

    Ok(Json(SessionRepo {
        id: updated.id,
        publication_state: updated.publication_state,
        role: Some(RepoRole::Owner),
    }))
}

async fn update_file_visibility(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((owner, repo_name)): Path<(String, String)>,
    Json(input): Json<UpdateFileVisibilityRequest>,
) -> Result<Json<RepoFileResponse>, ApiError> {
    let identity = http_identity(&state, &headers).await?;
    let path = ScopePath::parse(&input.path).map_err(ApiError::bad_request)?;
    let repo_id = scope_store::repo_id(&owner, &repo_name);

    let repo = find_repo(&state, &owner, &repo_name)?;
    let principal = principal_for_repo(&state, &repo, identity.as_ref())?;
    ensure_repo_read(&state, &repo, &principal)?;
    if role_for_principal(&state, &repo, &principal)? != Some(RepoRole::Owner) {
        return Err(ApiError::forbidden("owner role required"));
    }

    let owner_files = files_for_visibility_update(&repo, &principal)?;
    let selected_file = owner_files
        .iter()
        .find(|file| file.path == path.as_str())
        .ok_or_else(|| ApiError::not_found(format!("file {} not found", path.as_str())))?;
    if input.visibility == Visibility::Public && !selected_file.tracked {
        return Err(ApiError::bad_request(format!(
            "file {} must be tracked by Git before it can be made public",
            path.as_str()
        )));
    }

    let updated = {
        let mut catalog = lock_catalog(&state)?;
        let mut staged = catalog.clone();
        let repo = staged
            .repositories
            .get(&repo_id)
            .ok_or_else(|| ApiError::not_found(format!("repo {owner}/{repo_name} not found")))?;
        let user_id = identity
            .as_ref()
            .map(identity_user_id)
            .ok_or_else(|| ApiError::forbidden("owner role required"))?;
        let principal = principal_for_user_id(repo, &user_id);
        let role = staged.role_for_principal(repo, &principal);

        if role != Some(RepoRole::Owner) {
            return Err(ApiError::forbidden("owner role required"));
        }

        if input.visibility == Visibility::Public && !repo_has_file_for_review(repo, &path) {
            return Err(ApiError::bad_request(format!(
                "file {} must be tracked by Git before it can be made public",
                path.as_str()
            )));
        }

        {
            let repo = staged
                .repositories
                .get_mut(&repo_id)
                .expect("repo was already checked");
            let rule = match input.visibility {
                Visibility::Public => VisibilityRule::public(path.clone()),
                Visibility::Private => VisibilityRule::private(path.clone(), repo_owner_ids(repo)),
            };
            repo.policy.add_rule(rule).map_err(ApiError::bad_request)?;
        }

        persist_catalog(&state, &staged)?;
        let updated = staged
            .repositories
            .get(&repo_id)
            .expect("repo was already checked")
            .clone();
        *catalog = staged;
        updated
    };

    let principal = Principal {
        id: updated.record.owner_user_id.clone(),
        kind: scope_policy::PrincipalKind::User,
    };
    let updated_files = files_for_visibility_update(&updated, &principal)?;
    let file = updated_files
        .into_iter()
        .find(|file| file.path == path.as_str())
        .ok_or_else(|| ApiError::not_found(format!("file {} not found", path.as_str())))?;

    Ok(Json(file))
}

async fn get_settings(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((owner, repo_name)): Path<(String, String)>,
) -> Result<Json<RepoSettings>, ApiError> {
    let repo = find_repo(&state, &owner, &repo_name)?;
    let identity = http_identity(&state, &headers).await?;
    let principal = principal_for_repo(&state, &repo, identity.as_ref())?;
    ensure_repo_read(&state, &repo, &principal)?;

    if role_for_principal(&state, &repo, &principal)? != Some(RepoRole::Owner) {
        return Err(ApiError::forbidden("owner role required"));
    }

    Ok(Json(repo.settings))
}

async fn update_settings(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((owner, repo_name)): Path<(String, String)>,
    Json(input): Json<UpdateRepoSettingsRequest>,
) -> Result<Json<RepoSettings>, ApiError> {
    let identity = http_identity(&state, &headers).await?;
    let repo_id = scope_store::repo_id(&owner, &repo_name);
    let repo = find_repo(&state, &owner, &repo_name)?;
    let principal = principal_for_repo(&state, &repo, identity.as_ref())?;
    ensure_repo_read(&state, &repo, &principal)?;

    let mut catalog = lock_catalog(&state)?;
    let mut staged = catalog.clone();
    let repo = staged
        .repositories
        .get(&repo_id)
        .ok_or_else(|| ApiError::not_found(format!("repo {owner}/{repo_name} not found")))?;
    let user_id = identity
        .as_ref()
        .map(identity_user_id)
        .ok_or_else(|| ApiError::forbidden("owner role required"))?;
    let principal = principal_for_user_id(repo, &user_id);

    if staged.role_for_principal(repo, &principal) != Some(RepoRole::Owner) {
        return Err(ApiError::forbidden("owner role required"));
    }

    {
        let repo = staged
            .repositories
            .get_mut(&repo_id)
            .expect("repo was already checked");
        repo.settings.include_ignored_files = input.include_ignored_files;
    }

    persist_catalog(&state, &staged)?;
    let settings = staged
        .repositories
        .get(&repo_id)
        .expect("repo was already checked")
        .settings;
    *catalog = staged;

    Ok(Json(settings))
}

async fn git_info_refs(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((org, repo)): Path<(String, String)>,
    Query(query): Query<GitInfoRefsQuery>,
) -> Response {
    match query.service.as_deref() {
        Some("git-receive-pack") => {
            match handle_git_receive_pack(&state, &headers, &org, &repo, "GET", Vec::new(), None) {
                Ok(response) => response,
                Err(error) => git_error_response(error),
            }
        }
        Some("git-upload-pack") => (
            StatusCode::NOT_IMPLEMENTED,
            Json(serde_json::json!({
                "error": "git clone is blocked until publish creates a public Git projection"
            })),
        )
            .into_response(),
        Some(service) => (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({
                "error": format!("unsupported Git service {service}")
            })),
        )
            .into_response(),
        None => (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({
                "error": "missing Git service"
            })),
        )
            .into_response(),
    }
}

async fn git_receive_pack(
    State(state): State<AppState>,
    Path((org, repo)): Path<(String, String)>,
    request: Request,
) -> Response {
    let headers = request.headers().clone();
    let token_secret = match first_push_token_from_headers(&headers) {
        Ok(token_secret) => token_secret,
        Err(error) => return git_error_response(error),
    };
    if let Err(error) = authorize_first_push(&state, &org, &repo, &token_secret) {
        return git_error_response(error);
    }

    let content_type = headers
        .get(CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .map(str::to_string);
    let body = match to_bytes(request.into_body(), MAX_RECEIVE_PACK_BYTES).await {
        Ok(body) => body,
        Err(error) => {
            return (
                StatusCode::PAYLOAD_TOO_LARGE,
                Json(serde_json::json!({
                    "error": format!("git receive-pack body is too large: {error}")
                })),
            )
                .into_response();
        }
    };

    match handle_git_receive_pack(
        &state,
        &headers,
        &org,
        &repo,
        "POST",
        body.to_vec(),
        content_type,
    ) {
        Ok(response) => response,
        Err(error) => git_error_response(error),
    }
}

fn handle_git_receive_pack(
    state: &AppState,
    headers: &HeaderMap,
    owner: &str,
    repo_name: &str,
    method: &str,
    body: Vec<u8>,
    content_type: Option<String>,
) -> Result<Response, ApiError> {
    let token_secret = first_push_token_from_headers(headers)?;
    authorize_first_push(state, owner, repo_name, &token_secret)?;
    let staging_repo = ensure_receive_pack_staging_repo(state, owner, repo_name)?;
    let cgi = git_http_backend(
        &staging_repo,
        method,
        if method == "GET" {
            "info/refs"
        } else {
            "git-receive-pack"
        },
        if method == "GET" {
            "service=git-receive-pack"
        } else {
            ""
        },
        body,
        content_type,
    )?;

    if method == "POST" && cgi.status.is_success() {
        let import = match pending_import_from_staging_repo(&staging_repo) {
            Ok(import) => import,
            Err(error) => {
                let _ = fs::remove_dir_all(&staging_repo);
                return Err(error);
            }
        };
        if let Err(error) = persist_pending_import(state, owner, repo_name, &token_secret, import) {
            let _ = fs::remove_dir_all(&staging_repo);
            return Err(error);
        }
        let _ = fs::remove_dir_all(&staging_repo);
    }

    Ok(cgi.into_response())
}

fn first_push_token_from_headers(headers: &HeaderMap) -> Result<String, ApiError> {
    let Some(value) = headers.get(AUTHORIZATION) else {
        return Err(ApiError::unauthorized("first-push token required"));
    };
    let value = value
        .to_str()
        .map_err(|_| ApiError::unauthorized("invalid authorization header"))?;

    if let Some(token) = value.strip_prefix("Bearer ") {
        let token = token.trim();
        if token.is_empty() {
            return Err(ApiError::unauthorized("empty first-push token"));
        }
        return Ok(token.to_string());
    }

    if let Some(encoded) = value.strip_prefix("Basic ") {
        let decoded = BASE64
            .decode(encoded.trim())
            .map_err(|_| ApiError::unauthorized("invalid basic authorization"))?;
        let decoded = String::from_utf8(decoded)
            .map_err(|_| ApiError::unauthorized("invalid basic authorization"))?;
        let (username, password) = decoded.split_once(':').unwrap_or((decoded.as_str(), ""));
        let token = if password.is_empty() {
            username.trim()
        } else {
            password.trim()
        };
        if token.is_empty() {
            return Err(ApiError::unauthorized("empty first-push token"));
        }
        return Ok(token.to_string());
    }

    Err(ApiError::unauthorized(
        "expected Authorization: Basic or Bearer first-push token",
    ))
}

fn authorize_first_push(
    state: &AppState,
    owner: &str,
    repo_name: &str,
    token_secret: &str,
) -> Result<(), ApiError> {
    let repo = find_repo(state, owner, repo_name)?;
    if repo.record.publication_state != RepoPublicationState::PendingFirstPush {
        return Err(ApiError::conflict(
            "repo is not waiting for an initial Git push",
        ));
    }
    if repo.pending_import.is_some() {
        return Err(ApiError::conflict("repo already has a pending import"));
    }

    let now = unix_now()?;
    let Some(token) = repo.first_push_token.as_ref() else {
        return Err(ApiError::unauthorized("first-push token is not configured"));
    };
    if token.owner_user_id != repo.record.owner_user_id {
        return Err(ApiError::forbidden(
            "first-push token owner does not match repo owner",
        ));
    }
    if token.status_at(now) != FirstPushTokenStatus::Active {
        return Err(ApiError::unauthorized(
            "first-push token is expired or used",
        ));
    }
    if token.token_hash != first_push_token_hash(token_secret) {
        return Err(ApiError::unauthorized("invalid first-push token"));
    }

    Ok(())
}

fn ensure_receive_pack_staging_repo(
    state: &AppState,
    owner: &str,
    repo_name: &str,
) -> Result<PathBuf, ApiError> {
    let base_dir = state
        .state_path
        .parent()
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir);
    let repo_root = base_dir
        .join("git-receive")
        .join(format!("{}.git", safe_repo_key(owner, repo_name)));
    if repo_root.join("HEAD").exists() {
        match git_refs(&repo_root) {
            Ok(refs) if !refs.is_empty() => {
                fs::remove_dir_all(&repo_root).map_err(ApiError::internal)?;
            }
            Err(_) => {
                fs::remove_dir_all(&repo_root).map_err(ApiError::internal)?;
            }
            _ => {}
        }
    }
    if !repo_root.join("HEAD").exists() {
        if let Some(parent) = repo_root.parent() {
            fs::create_dir_all(parent).map_err(ApiError::internal)?;
        }
        run_git(
            None,
            &["init", "--bare", repo_root.to_string_lossy().as_ref()],
            "initializing receive-pack staging repo",
        )?;
        run_git(
            Some(&repo_root),
            &["config", "http.receivepack", "true"],
            "enabling receive-pack",
        )?;
        install_pre_receive_hook(&repo_root)?;
    }
    Ok(repo_root)
}

fn install_pre_receive_hook(repo_root: &FsPath) -> Result<(), ApiError> {
    let hook = repo_root.join("hooks").join("pre-receive");
    let script = format!(
        "#!/bin/sh\ncount=0\nwhile read old new ref; do\n  count=$((count + 1))\n  case \"$ref\" in\n    refs/heads/*) ;;\n    *) echo \"Scope accepts only the first pushed branch in v0\" >&2; exit 1 ;;\n  esac\n  if [ \"$new\" = \"{EMPTY_GIT_OID}\" ]; then\n    echo \"Scope does not accept branch deletes in v0\" >&2\n    exit 1\n  fi\n  if [ \"$old\" != \"{EMPTY_GIT_OID}\" ]; then\n    echo \"Scope accepts only the initial branch push in v0\" >&2\n    exit 1\n  fi\ndone\nif [ \"$count\" -ne 1 ]; then\n  echo \"Scope accepts exactly one pushed branch in v0\" >&2\n  exit 1\nfi\n"
    );
    fs::write(&hook, script).map_err(ApiError::internal)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut permissions = fs::metadata(&hook)
            .map_err(ApiError::internal)?
            .permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(&hook, permissions).map_err(ApiError::internal)?;
    }
    Ok(())
}

fn git_http_backend(
    staging_repo: &FsPath,
    method: &str,
    path_suffix: &str,
    query_string: &str,
    body: Vec<u8>,
    content_type: Option<String>,
) -> Result<CgiResponse, ApiError> {
    let staging_parent = staging_repo
        .parent()
        .ok_or_else(|| ApiError::internal_message("staging repo is missing a parent"))?;
    let repo_name = staging_repo
        .file_name()
        .and_then(|value| value.to_str())
        .ok_or_else(|| ApiError::internal_message("staging repo has invalid path"))?;
    let mut command = Command::new("git");
    command
        .arg("http-backend")
        .env("GIT_PROJECT_ROOT", staging_parent)
        .env("GIT_HTTP_EXPORT_ALL", "1")
        .env("REQUEST_METHOD", method)
        .env("PATH_INFO", format!("/{repo_name}/{path_suffix}"))
        .env("QUERY_STRING", query_string)
        .env("REMOTE_USER", "first-push-token")
        .env("CONTENT_LENGTH", body.len().to_string())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    if let Some(content_type) = content_type {
        command.env("CONTENT_TYPE", content_type);
    }

    let mut child = command.spawn().map_err(|error| {
        ApiError::service_unavailable(format!("failed to start git http-backend: {error}"))
    })?;
    if let Some(stdin) = child.stdin.as_mut() {
        stdin.write_all(&body).map_err(ApiError::internal)?;
    }

    let output = child.wait_with_output().map_err(ApiError::internal)?;
    if !output.status.success() {
        return Err(ApiError::service_unavailable(format!(
            "git http-backend failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        )));
    }

    CgiResponse::parse(output.stdout)
}

struct CgiResponse {
    status: StatusCode,
    headers: Vec<(String, String)>,
    body: Vec<u8>,
}

impl CgiResponse {
    fn parse(output: Vec<u8>) -> Result<Self, ApiError> {
        let header_end = find_header_end(&output)
            .ok_or_else(|| ApiError::service_unavailable("git http-backend returned no headers"))?;
        let (headers, body) = output.split_at(header_end.0);
        let headers = String::from_utf8_lossy(headers);
        let mut status = StatusCode::OK;
        let mut parsed_headers = Vec::new();

        for line in headers
            .lines()
            .map(str::trim)
            .filter(|line| !line.is_empty())
        {
            let Some((name, value)) = line.split_once(':') else {
                continue;
            };
            if name.eq_ignore_ascii_case("Status") {
                let code = value
                    .trim()
                    .split_whitespace()
                    .next()
                    .and_then(|code| code.parse::<u16>().ok())
                    .ok_or_else(|| ApiError::service_unavailable("invalid git CGI status"))?;
                status = StatusCode::from_u16(code).map_err(ApiError::internal)?;
            } else {
                parsed_headers.push((name.trim().to_string(), value.trim().to_string()));
            }
        }

        Ok(Self {
            status,
            headers: parsed_headers,
            body: body[header_end.1..].to_vec(),
        })
    }

    fn into_response(self) -> Response {
        let mut builder = Response::builder().status(self.status);
        for (name, value) in self.headers {
            builder = builder.header(name, value);
        }
        builder
            .body(Body::from(self.body))
            .expect("git CGI response headers should be valid")
    }
}

fn find_header_end(bytes: &[u8]) -> Option<(usize, usize)> {
    bytes
        .windows(4)
        .position(|window| window == b"\r\n\r\n")
        .map(|index| (index, 4))
        .or_else(|| {
            bytes
                .windows(2)
                .position(|window| window == b"\n\n")
                .map(|index| (index, 2))
        })
}

fn pending_import_from_staging_repo(staging_repo: &FsPath) -> Result<PendingImport, ApiError> {
    let refs = git_refs(staging_repo)?;
    if refs.len() != 1 {
        return Err(ApiError::bad_request(
            "push must create exactly one branch and no tags",
        ));
    }
    let (refname, head_oid) = refs.into_iter().next().expect("length checked");
    let Some(default_branch) = refname.strip_prefix("refs/heads/") else {
        return Err(ApiError::bad_request("only branch pushes are supported"));
    };
    validate_branch_name(default_branch)?;
    let tree_oid = git_stdout_text(
        staging_repo,
        &["rev-parse", &format!("{head_oid}^{{tree}}")],
        "reading pushed tree",
    )?
    .trim()
    .to_string();
    let files = git_tree_files(staging_repo, &head_oid)?;

    Ok(PendingImport {
        default_branch: default_branch.to_string(),
        head_oid,
        tree_oid,
        imported_at_unix: unix_now()?,
        files,
    })
}

fn git_refs(staging_repo: &FsPath) -> Result<Vec<(String, String)>, ApiError> {
    let output = run_git_output(
        Some(staging_repo),
        &[
            "for-each-ref",
            "--format=%(refname)%00%(objectname)",
            "refs",
        ],
        "reading pushed refs",
    )?;
    let text = String::from_utf8(output.stdout).map_err(ApiError::bad_request)?;
    text.lines()
        .map(|line| {
            let (refname, oid) = line
                .split_once('\0')
                .ok_or_else(|| ApiError::internal_message("invalid git ref listing"))?;
            Ok((refname.to_string(), oid.to_string()))
        })
        .collect()
}

fn validate_branch_name(branch: &str) -> Result<(), ApiError> {
    if branch.is_empty() || branch.starts_with('-') || branch.contains("..") {
        return Err(ApiError::bad_request("invalid branch name"));
    }
    run_git(
        None,
        &["check-ref-format", &format!("refs/heads/{branch}")],
        "validating branch name",
    )
}

fn git_tree_files(
    staging_repo: &FsPath,
    head_oid: &str,
) -> Result<Vec<PendingImportFile>, ApiError> {
    let output = run_git_output(
        Some(staging_repo),
        &["ls-tree", "-rz", "-r", head_oid],
        "reading pushed tree",
    )?;
    let mut files = Vec::new();
    let mut total_bytes = 0usize;
    for raw in output.stdout.split(|byte| *byte == 0) {
        if raw.is_empty() {
            continue;
        }
        if files.len() >= MAX_PENDING_IMPORT_FILES {
            return Err(ApiError::bad_request(format!(
                "pending import exceeds {MAX_PENDING_IMPORT_FILES} files"
            )));
        }
        let entry = std::str::from_utf8(raw).map_err(ApiError::bad_request)?;
        let Some((metadata, path)) = entry.split_once('\t') else {
            return Err(ApiError::internal_message("invalid git tree entry"));
        };
        let mut fields = metadata.split_whitespace();
        let mode = fields
            .next()
            .ok_or_else(|| ApiError::internal_message("tree entry is missing mode"))?;
        let kind = fields
            .next()
            .ok_or_else(|| ApiError::internal_message("tree entry is missing type"))?;
        let oid = fields
            .next()
            .ok_or_else(|| ApiError::internal_message("tree entry is missing oid"))?;
        if kind != "blob" {
            return Err(ApiError::bad_request(format!(
                "unsupported Git tree entry {path}: {kind}"
            )));
        }
        validate_pushed_file_path(path)?;
        if mode != "100644" {
            return Err(ApiError::bad_request(format!(
                "unsupported Git file mode {path}: {mode}"
            )));
        }
        let blob_size = git_stdout_text(
            staging_repo,
            &["cat-file", "-s", oid],
            "reading pushed blob size",
        )?
        .trim()
        .parse::<usize>()
        .map_err(|_| ApiError::internal_message("invalid Git blob size"))?;
        if blob_size > MAX_PENDING_IMPORT_BLOB_BYTES {
            return Err(ApiError::bad_request(format!(
                "blob {path} is larger than {MAX_PENDING_IMPORT_BLOB_BYTES} bytes"
            )));
        }
        total_bytes = total_bytes
            .checked_add(blob_size)
            .ok_or_else(|| ApiError::bad_request("pending import is too large"))?;
        if total_bytes > MAX_PENDING_IMPORT_TOTAL_BYTES {
            return Err(ApiError::bad_request(format!(
                "pending import exceeds {MAX_PENDING_IMPORT_TOTAL_BYTES} bytes"
            )));
        }
        let content = run_git_output(
            Some(staging_repo),
            &["cat-file", "blob", oid],
            "reading pushed blob",
        )?
        .stdout;
        std::str::from_utf8(&content)
            .map_err(|_| ApiError::bad_request(format!("blob {path} must be valid UTF-8 text")))?;
        files.push(PendingImportFile {
            path: path.to_string(),
            mode: mode.to_string(),
            oid: oid.to_string(),
            content_base64: BASE64.encode(content),
        });
    }
    files.sort_by(|left, right| left.path.cmp(&right.path));
    Ok(files)
}

fn validate_pushed_file_path(path: &str) -> Result<(), ApiError> {
    if path.is_empty() || path.starts_with('/') || path.contains('\\') {
        return Err(ApiError::bad_request(format!(
            "unsupported Git file path {path:?}"
        )));
    }
    if path.bytes().any(|byte| byte < 0x20 || byte == 0x7f) {
        return Err(ApiError::bad_request(format!(
            "unsupported Git file path {path:?}"
        )));
    }

    let scope_path = ScopePath::parse(format!("/{path}")).map_err(ApiError::bad_request)?;
    if scope_path.as_str() != format!("/{path}") {
        return Err(ApiError::bad_request(format!(
            "unsupported Git file path {path:?}"
        )));
    }

    Ok(())
}

fn persist_pending_import(
    state: &AppState,
    owner: &str,
    repo_name: &str,
    token_secret: &str,
    import: PendingImport,
) -> Result<(), ApiError> {
    let repo_id = scope_store::repo_id(owner, repo_name);
    let now = unix_now()?;
    let mut catalog = lock_catalog(state)?;
    let mut staged = catalog.clone();
    {
        let repo = staged
            .repositories
            .get_mut(&repo_id)
            .ok_or_else(|| ApiError::not_found(format!("repo {owner}/{repo_name} not found")))?;
        if repo.record.publication_state != RepoPublicationState::PendingFirstPush {
            return Err(ApiError::conflict(
                "repo is not waiting for an initial Git push",
            ));
        }
        if repo.pending_import.is_some() {
            return Err(ApiError::conflict("repo already has a pending import"));
        }
        let Some(token) = repo.first_push_token.as_mut() else {
            return Err(ApiError::unauthorized("first-push token is not configured"));
        };
        if token.status_at(now) != FirstPushTokenStatus::Active
            || token.token_hash != first_push_token_hash(token_secret)
        {
            return Err(ApiError::unauthorized(
                "first-push token is expired, used, or invalid",
            ));
        }
        token.used_at_unix = Some(now);
        repo.pending_import = Some(import);
        repo.record.publication_state = RepoPublicationState::PendingPublish;
    }

    persist_catalog(state, &staged)?;
    *catalog = staged;
    Ok(())
}

fn run_git(repo: Option<&FsPath>, args: &[&str], action: &str) -> Result<(), ApiError> {
    let output = run_git_output(repo, args, action)?;
    if output.status.success() {
        Ok(())
    } else {
        Err(ApiError::service_unavailable(format!(
            "{action}: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        )))
    }
}

fn git_stdout_text(repo: &FsPath, args: &[&str], action: &str) -> Result<String, ApiError> {
    let output = run_git_output(Some(repo), args, action)?;
    if !output.status.success() {
        return Err(ApiError::service_unavailable(format!(
            "{action}: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        )));
    }
    String::from_utf8(output.stdout).map_err(ApiError::bad_request)
}

fn run_git_output(
    repo: Option<&FsPath>,
    args: &[&str],
    action: &str,
) -> Result<std::process::Output, ApiError> {
    let mut command = Command::new("git");
    if let Some(repo) = repo {
        command.arg("-C").arg(repo);
    }
    command
        .args(args)
        .output()
        .map_err(|error| ApiError::service_unavailable(format!("failed {action}: {error}")))
}

fn safe_repo_key(owner: &str, repo_name: &str) -> String {
    format!(
        "{}-{}",
        owner
            .chars()
            .map(|ch| if ch.is_ascii_alphanumeric() { ch } else { '-' })
            .collect::<String>(),
        repo_name
            .chars()
            .map(|ch| if ch.is_ascii_alphanumeric() { ch } else { '-' })
            .collect::<String>()
    )
}

fn git_error_response(error: ApiError) -> Response {
    if error.status == StatusCode::UNAUTHORIZED {
        let mut response = error.into_response();
        response.headers_mut().insert(
            WWW_AUTHENTICATE,
            HeaderValue::from_static("Basic realm=\"Scope first push\""),
        );
        return response;
    }
    error.into_response()
}

fn find_repo(state: &AppState, owner: &str, name: &str) -> Result<StoredRepository, ApiError> {
    lock_catalog(state)?
        .repository(owner, name)
        .cloned()
        .ok_or_else(|| ApiError::not_found(format!("repo {owner}/{name} not found")))
}

fn ensure_repo_read(
    state: &AppState,
    repo: &StoredRepository,
    principal: &Principal,
) -> Result<(), ApiError> {
    if can_read_path(state, repo, principal, &ScopePath::root())? {
        Ok(())
    } else {
        Err(ApiError::not_found(format!(
            "repo {} not found",
            repo.record.id
        )))
    }
}

fn ensure_owner(
    state: &AppState,
    repo: &StoredRepository,
    principal: &Principal,
) -> Result<(), ApiError> {
    if role_for_principal(state, repo, principal)? == Some(RepoRole::Owner) {
        Ok(())
    } else {
        Err(ApiError::forbidden("owner role required"))
    }
}

fn ensure_pending_publish(repo: &StoredRepository) -> Result<(), ApiError> {
    if repo.record.publication_state != RepoPublicationState::PendingPublish {
        return Err(ApiError::bad_request("repo is not pending publish"));
    }
    if repo.pending_import.is_none() {
        return Err(ApiError::bad_request(
            "repo has no pending import to publish",
        ));
    }
    Ok(())
}

fn promote_pending_import(repo: &mut StoredRepository) -> Result<(), ApiError> {
    ensure_pending_publish(repo)?;
    let pending = repo
        .pending_import
        .take()
        .ok_or_else(|| ApiError::bad_request("repo has no pending import to publish"))?;
    let changes = pending_import_changes(&pending)?;
    let parent_ids = repo
        .graph
        .commits
        .last()
        .map(|commit| vec![commit.id.clone()])
        .unwrap_or_default();
    let logical_id = format!(
        "rv_git_{}",
        pending
            .head_oid
            .get(..12)
            .unwrap_or(pending.head_oid.as_str())
    );
    repo.graph.commits.push(LogicalCommit {
        id: logical_id,
        parent_ids,
        author_id: repo.record.owner_user_id.clone(),
        author_visibility: AuthorVisibility::Visible,
        message: format!("Import pushed {}", pending.default_branch),
        mixed_policy: MixedCommitPolicy::SyntheticPublicCommit,
        changes,
    });
    repo.record.publication_state = RepoPublicationState::Published;
    Ok(())
}

impl AppState {
    fn from_env() -> anyhow::Result<Self> {
        let repo_root = git_repo_root();
        let state_path = state_path(&repo_root);
        let persisted_state = load_state(&state_path)?;
        let mut catalog = app_catalog();
        apply_persisted_state(&mut catalog, &persisted_state);

        Ok(Self {
            catalog: Arc::new(Mutex::new(catalog)),
            state_path: Arc::new(state_path),
            shoo: ShooVerifier::from_env(),
        })
    }

    #[cfg(test)]
    fn test_state() -> Self {
        Self {
            catalog: Arc::new(Mutex::new(app_catalog())),
            state_path: Arc::new(test_state_path()),
            shoo: ShooVerifier::new(
                SHOO_ISSUER,
                Some("origin:http://localhost:3000".to_string()),
                "http://127.0.0.1/.well-known/jwks.json",
            ),
        }
    }
}

#[cfg(test)]
fn test_state_path() -> PathBuf {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("test clock must be after UNIX epoch")
        .as_nanos();
    std::env::temp_dir().join(format!(
        "scope-vcs-test-state-{}-{nanos}.json",
        std::process::id()
    ))
}

fn non_empty_env(name: &str) -> Option<String> {
    std::env::var(name).ok().filter(|value| !value.is_empty())
}

fn state_path(repo_root: &FsPath) -> PathBuf {
    non_empty_env(SCOPE_STATE_PATH_ENV)
        .map(|value| {
            let path = PathBuf::from(value);
            if path.is_absolute() {
                path
            } else {
                repo_root.join(path)
            }
        })
        .unwrap_or_else(|| repo_root.join(".scope").join("state.json"))
}

fn load_state(path: &FsPath) -> anyhow::Result<PersistedState> {
    let bytes = match fs::read(path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(PersistedState::default());
        }
        Err(error) => {
            return Err(error).with_context(|| format!("reading {}", path.display()));
        }
    };
    serde_json::from_slice(&bytes).with_context(|| format!("parsing {}", path.display()))
}

fn persist_catalog(state: &AppState, catalog: &AppCatalog) -> Result<(), ApiError> {
    if let Some(parent) = state.state_path.parent() {
        fs::create_dir_all(parent).map_err(ApiError::internal)?;
    }

    let bytes = serde_json::to_vec_pretty(&persisted_state_from_catalog(catalog))
        .map_err(ApiError::internal)?;
    let temp_path = state
        .state_path
        .with_extension(format!("json.{}.tmp", std::process::id()));
    {
        let mut file = fs::File::create(&temp_path).map_err(ApiError::internal)?;
        file.write_all(&bytes).map_err(ApiError::internal)?;
        file.sync_all().map_err(ApiError::internal)?;
    }

    fs::rename(&temp_path, state.state_path.as_ref()).map_err(|error| {
        let _ = fs::remove_file(&temp_path);
        ApiError::internal(error)
    })?;

    Ok(())
}

fn apply_persisted_state(catalog: &mut AppCatalog, state: &PersistedState) {
    catalog.users = state.users.clone();
    catalog.repositories = state.repositories.clone();
}

fn persisted_state_from_catalog(catalog: &AppCatalog) -> PersistedState {
    PersistedState {
        users: catalog.users.clone(),
        repositories: catalog.repositories.clone(),
    }
}

impl ShooVerifier {
    fn from_env() -> Self {
        Self::new(SHOO_ISSUER, shoo_audience_from_env(), SHOO_JWKS_URL)
    }

    fn new(
        issuer: impl Into<String>,
        audience: Option<String>,
        jwks_url: impl Into<String>,
    ) -> Self {
        Self {
            client: reqwest::Client::builder()
                .timeout(Duration::from_secs(5))
                .build()
                .expect("Shoo verifier HTTP client config must be valid"),
            issuer: issuer.into(),
            audience,
            jwks_url: jwks_url.into(),
            jwks_cache: Arc::new(Mutex::new(None)),
        }
    }

    async fn verify(&self, token: &str) -> Result<ShooIdentity, ApiError> {
        validated_shoo_header(token)?;
        let audience = self.audience.as_deref().ok_or_else(|| {
            ApiError::service_unavailable(format!(
                "Shoo auth requires {SCOPE_APP_ORIGIN_ENV} to match the web app origin"
            ))
        })?;
        let jwks = self.jwks().await?;

        verify_shoo_token(token, &jwks, &self.issuer, audience)
    }

    async fn jwks(&self) -> Result<JwkSet, ApiError> {
        if let Some(jwks) = self
            .jwks_cache
            .lock()
            .expect("Shoo JWKS cache lock must not be poisoned")
            .clone()
        {
            return Ok(jwks);
        }

        let jwks = self
            .client
            .get(&self.jwks_url)
            .send()
            .await
            .map_err(|error| {
                ApiError::service_unavailable(format!("failed to fetch Shoo JWKS: {error}"))
            })?
            .error_for_status()
            .map_err(|error| {
                ApiError::service_unavailable(format!("failed to fetch Shoo JWKS: {error}"))
            })?
            .json::<JwkSet>()
            .await
            .map_err(ApiError::internal)?;

        *self
            .jwks_cache
            .lock()
            .expect("Shoo JWKS cache lock must not be poisoned") = Some(jwks.clone());
        Ok(jwks)
    }
}

fn shoo_audience_from_env() -> Option<String> {
    let app_origin = non_empty_env(SCOPE_APP_ORIGIN_ENV)
        .or_else(|| cfg!(debug_assertions).then(|| LOCAL_APP_ORIGIN.to_string()))?;

    Some(format!("origin:{}", app_origin.trim_end_matches('/')))
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct ShooIdentity {
    pairwise_sub: String,
    email: Option<String>,
    email_verified: bool,
}

impl From<&ShooIdentity> for SessionIdentity {
    fn from(identity: &ShooIdentity) -> Self {
        Self {
            pairwise_sub: identity.pairwise_sub.clone(),
            email: identity.email.clone(),
            email_verified: identity.email_verified,
        }
    }
}

#[derive(Debug, Deserialize, Serialize)]
struct ShooClaims {
    pairwise_sub: String,
    email: Option<String>,
    email_verified: Option<bool>,
}

fn verify_shoo_token(
    token: &str,
    jwks: &JwkSet,
    issuer: &str,
    audience: &str,
) -> Result<ShooIdentity, ApiError> {
    let header = validated_shoo_header(token)?;
    let jwk = signing_key(&header.kid, jwks)?;
    let key = DecodingKey::from_jwk(jwk).map_err(ApiError::internal)?;
    let mut validation = Validation::new(Algorithm::ES256);
    validation.set_required_spec_claims(&["exp", "iss", "aud"]);
    validation.set_issuer(&[issuer]);
    validation.set_audience(&[audience]);

    let claims = decode::<ShooClaims>(token, &key, &validation)
        .map_err(|_| ApiError::unauthorized("invalid Shoo token"))?
        .claims;

    if claims.pairwise_sub.trim().is_empty() {
        return Err(ApiError::unauthorized("Shoo token is missing pairwise_sub"));
    }

    Ok(ShooIdentity {
        pairwise_sub: claims.pairwise_sub,
        email: claims.email,
        email_verified: claims.email_verified.unwrap_or(false),
    })
}

fn validated_shoo_header(token: &str) -> Result<jsonwebtoken::Header, ApiError> {
    let header =
        decode_header(token).map_err(|_| ApiError::unauthorized("invalid bearer token"))?;
    if header.alg != Algorithm::ES256 {
        return Err(ApiError::unauthorized("unsupported Shoo token algorithm"));
    }
    if header.kid.is_none() {
        return Err(ApiError::unauthorized("Shoo token is missing kid"));
    }

    Ok(header)
}

fn signing_key<'a>(kid: &Option<String>, jwks: &'a JwkSet) -> Result<&'a Jwk, ApiError> {
    let Some(kid) = kid.as_deref() else {
        return Err(ApiError::unauthorized("Shoo token is missing kid"));
    };

    jwks.keys
        .iter()
        .find(|jwk| jwk.common.key_id.as_deref() == Some(kid))
        .ok_or_else(|| ApiError::unauthorized("Shoo signing key not found"))
}

async fn http_identity(
    state: &AppState,
    headers: &HeaderMap,
) -> Result<Option<ShooIdentity>, ApiError> {
    let Some(token) = bearer_token(headers)? else {
        return Ok(None);
    };

    state.shoo.verify(token).await.map(Some)
}

async fn require_identity(state: &AppState, headers: &HeaderMap) -> Result<ShooIdentity, ApiError> {
    http_identity(state, headers)
        .await?
        .ok_or_else(|| ApiError::unauthorized("sign in required"))
}

fn bearer_token(headers: &HeaderMap) -> Result<Option<&str>, ApiError> {
    let Some(value) = headers.get(AUTHORIZATION) else {
        return Ok(None);
    };
    let raw = value
        .to_str()
        .map_err(|_| ApiError::unauthorized("invalid authorization header"))?;
    let Some(token) = raw.strip_prefix("Bearer ") else {
        return Err(ApiError::unauthorized(
            "expected Authorization: Bearer token",
        ));
    };
    if token.trim().is_empty() {
        return Err(ApiError::unauthorized("empty bearer token"));
    }

    Ok(Some(token.trim()))
}

fn principal_for_repo(
    state: &AppState,
    repo: &StoredRepository,
    identity: Option<&ShooIdentity>,
) -> Result<Principal, ApiError> {
    let Some(identity) = identity else {
        return Ok(Principal::public());
    };

    let user = ensure_user_for_identity(state, identity)?;
    Ok(principal_for_user_id(repo, &user.id))
}

fn ensure_user_for_identity(
    state: &AppState,
    identity: &ShooIdentity,
) -> Result<UserAccount, ApiError> {
    let user_id = identity_user_id(identity);
    let email = identity
        .email
        .as_deref()
        .map(normalize_email)
        .unwrap_or_default();

    let mut catalog = lock_catalog(state)?;
    let mut staged = catalog.clone();
    let user = match staged.users.get_mut(&user_id) {
        Some(user) => {
            user.email = email;
            user.email_verified = identity.email_verified;
            user.access = AccountAccess::Member;
            user.clone()
        }
        None => {
            let handle = unique_user_handle(&staged, &preferred_user_handle(identity), &user_id);
            let user = UserAccount {
                id: user_id.clone(),
                handle,
                email,
                email_verified: identity.email_verified,
                access: AccountAccess::Member,
            };
            staged.users.insert(user_id, user.clone());
            user
        }
    };

    persist_catalog(state, &staged)?;
    *catalog = staged;
    Ok(user)
}

fn principal_for_user_id(repo: &StoredRepository, user_id: &str) -> Principal {
    if repo
        .memberships
        .iter()
        .any(|membership| membership.user_id == user_id)
    {
        Principal {
            id: user_id.to_string(),
            kind: scope_policy::PrincipalKind::User,
        }
    } else {
        Principal::public()
    }
}

fn identity_user_id(identity: &ShooIdentity) -> String {
    let digest = Sha1::digest(identity.pairwise_sub.as_bytes());
    let hex = format!("{digest:x}");
    format!("user_{}", &hex[..16])
}

fn preferred_user_handle(identity: &ShooIdentity) -> String {
    let digest = Sha1::digest(identity.pairwise_sub.as_bytes());
    let hex = format!("{digest:x}");
    let fallback = format!("user-{}", &hex[..8]);
    let raw = identity
        .email
        .as_deref()
        .filter(|_| identity.email_verified)
        .and_then(|email| email.split('@').next())
        .filter(|local| !local.trim().is_empty())
        .unwrap_or(&fallback);

    normalize_handle(raw).unwrap_or(fallback)
}

fn unique_user_handle(catalog: &AppCatalog, preferred: &str, user_id: &str) -> String {
    let base = normalize_handle(preferred).unwrap_or_else(|| "user".to_string());
    if handle_is_available(catalog, &base, user_id) {
        return base;
    }

    for suffix in 2.. {
        let candidate = format!("{base}-{suffix}");
        if handle_is_available(catalog, &candidate, user_id) {
            return candidate;
        }
    }

    unreachable!("infinite suffix search must find an available handle")
}

fn handle_is_available(catalog: &AppCatalog, handle: &str, user_id: &str) -> bool {
    catalog
        .users
        .values()
        .all(|user| user.id == user_id || user.handle != handle)
}

fn normalize_handle(value: &str) -> Option<String> {
    let mut handle = String::new();
    let mut last_was_separator = false;
    for byte in value.trim().bytes() {
        let next = if byte.is_ascii_alphanumeric() {
            last_was_separator = false;
            Some(byte.to_ascii_lowercase() as char)
        } else if matches!(byte, b'-' | b'_') && !last_was_separator {
            last_was_separator = true;
            Some('-')
        } else {
            None
        };

        if let Some(next) = next {
            handle.push(next);
        }
    }

    let handle = handle.trim_matches('-').to_string();
    if handle.is_empty() || handle.len() > 40 {
        None
    } else {
        Some(handle)
    }
}

fn normalize_email(email: &str) -> String {
    email.trim().to_ascii_lowercase()
}

fn repo_summary(
    catalog: &AppCatalog,
    repo: &StoredRepository,
    user_id: &str,
) -> Option<RepoSummaryResponse> {
    let principal = Principal {
        id: user_id.to_string(),
        kind: scope_policy::PrincipalKind::User,
    };
    if !catalog.can_read_path(repo, &principal, &ScopePath::root()) {
        return None;
    }

    let role = catalog.role_for_principal(repo, &principal)?;

    Some(RepoSummaryResponse {
        id: repo.record.id.clone(),
        owner_handle: repo.record.owner_handle.clone(),
        name: repo.record.name.clone(),
        lifecycle_state: repo.record.publication_state,
        default_visibility: repo.record.default_visibility,
        role,
    })
}

fn repo_setup_response(
    catalog: &AppCatalog,
    repo: &StoredRepository,
    user_id: &str,
    now_unix: u64,
    secret: Option<String>,
) -> Result<RepoSetupResponse, ApiError> {
    ensure_owner_setup_access_in_catalog(catalog, repo, user_id)?;
    let repo = repo_summary(catalog, repo, user_id)
        .ok_or_else(|| ApiError::internal_message("setup repository is not readable"))?;
    let token = catalog
        .repositories
        .get(&repo.id)
        .and_then(|stored| stored.first_push_token.as_ref())
        .map(|stored_token| first_push_token_response(stored_token, now_unix, secret));

    Ok(RepoSetupResponse {
        git_remote_path: format!("/git/{}/{}", repo.owner_handle, repo.name),
        remote_name: "scope",
        push_branch: "<branch>",
        push_enabled: false,
        repo,
        token,
    })
}

fn first_push_token_response(
    token: &FirstPushToken,
    now_unix: u64,
    secret: Option<String>,
) -> FirstPushTokenResponse {
    FirstPushTokenResponse {
        status: token.status_at(now_unix),
        created_at_unix: token.created_at_unix,
        expires_at_unix: token.expires_at_unix,
        used_at_unix: token.used_at_unix,
        secret,
    }
}

fn ensure_owner_setup_access(
    state: &AppState,
    repo: &StoredRepository,
    user_id: &str,
) -> Result<(), ApiError> {
    let catalog = lock_catalog(state)?;
    ensure_owner_setup_access_in_catalog(&catalog, repo, user_id)
}

fn ensure_owner_setup_access_in_catalog(
    catalog: &AppCatalog,
    repo: &StoredRepository,
    user_id: &str,
) -> Result<(), ApiError> {
    let principal = Principal {
        id: user_id.to_string(),
        kind: scope_policy::PrincipalKind::User,
    };
    if catalog.role_for_principal(repo, &principal) != Some(RepoRole::Owner) {
        return Err(ApiError::not_found(format!(
            "repo {} not found",
            repo.record.id
        )));
    }
    if repo.record.publication_state != RepoPublicationState::PendingFirstPush {
        return Err(ApiError::conflict(
            "setup token is only available before the first push",
        ));
    }

    Ok(())
}

fn generate_first_push_token(owner_user_id: &str) -> Result<(String, FirstPushToken), ApiError> {
    let now = unix_now()?;
    let mut bytes = [0_u8; FIRST_PUSH_TOKEN_BYTES];
    getrandom::fill(&mut bytes).map_err(|error| {
        ApiError::internal_message(format!("failed to generate setup token: {error}"))
    })?;
    let secret = format!("scope_fp_{}", hex::encode(bytes));
    let token = FirstPushToken {
        token_hash: first_push_token_hash(&secret),
        owner_user_id: owner_user_id.to_string(),
        created_at_unix: now,
        expires_at_unix: now + FIRST_PUSH_TOKEN_TTL_SECS,
        used_at_unix: None,
    };

    Ok((secret, token))
}

fn first_push_token_hash(secret: &str) -> String {
    let digest = Sha256::digest(secret.as_bytes());
    format!("sha256:{digest:x}")
}

fn unix_now() -> Result<u64, ApiError> {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .map_err(ApiError::internal)
}

fn catalog_error(error: CatalogError) -> ApiError {
    match error {
        CatalogError::InvalidRepositoryName(message) => ApiError::bad_request(message),
        CatalogError::RepositoryExists(repo) => {
            ApiError::conflict(format!("repo {repo} already exists"))
        }
    }
}

fn role_for_principal(
    state: &AppState,
    repo: &StoredRepository,
    principal: &Principal,
) -> Result<Option<RepoRole>, ApiError> {
    Ok(lock_catalog(state)?.role_for_principal(repo, principal))
}

fn can_read_path(
    state: &AppState,
    repo: &StoredRepository,
    principal: &Principal,
    path: &ScopePath,
) -> Result<bool, ApiError> {
    Ok(lock_catalog(state)?.can_read_path(repo, principal, path))
}

fn can_write_path(
    state: &AppState,
    repo: &StoredRepository,
    principal: &Principal,
    path: &ScopePath,
) -> Result<bool, ApiError> {
    Ok(lock_catalog(state)?.can_write_path(repo, principal, path))
}

fn lock_catalog(state: &AppState) -> Result<std::sync::MutexGuard<'_, AppCatalog>, ApiError> {
    state
        .catalog
        .lock()
        .map_err(|_| ApiError::internal_message("catalog lock is poisoned"))
}

fn git_repo_root() -> PathBuf {
    if let Some(root) = non_empty_env(SCOPE_REPO_ROOT_ENV) {
        return PathBuf::from(root);
    }

    let output = Command::new("git")
        .args(["rev-parse", "--show-toplevel"])
        .output();
    if let Ok(output) = output
        && output.status.success()
        && let Ok(root) = String::from_utf8(output.stdout)
    {
        return PathBuf::from(root.trim());
    }

    std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."))
}

fn graph_has_file(repo: &StoredRepository, path: &ScopePath) -> bool {
    let mut present = false;
    for change in repo.graph.commits.iter().flat_map(|commit| &commit.changes) {
        if change.path.as_str() == path.as_str() {
            present = change.new_content.is_some();
        }
    }

    present
}

fn pending_import_has_file(repo: &StoredRepository, path: &ScopePath) -> bool {
    repo.pending_import.as_ref().is_some_and(|pending| {
        pending.files.iter().any(|file| {
            pending_scope_path(&file.path)
                .map(|pending_path| pending_path.as_str() == path.as_str())
                .unwrap_or(false)
        })
    })
}

fn repo_has_file_for_review(repo: &StoredRepository, path: &ScopePath) -> bool {
    if repo.record.publication_state == RepoPublicationState::PendingPublish {
        pending_import_has_file(repo, path)
    } else {
        graph_has_file(repo, path)
    }
}

fn repo_owner_ids(repo: &StoredRepository) -> Vec<String> {
    let mut owner_ids = repo
        .memberships
        .iter()
        .filter(|membership| membership.role == RepoRole::Owner)
        .map(|membership| membership.user_id.clone())
        .collect::<Vec<_>>();
    if !owner_ids.contains(&repo.record.owner_user_id) {
        owner_ids.push(repo.record.owner_user_id.clone());
    }
    owner_ids.sort();
    owner_ids.dedup();
    owner_ids
}

fn projected_files(repo: &StoredRepository, principal: &Principal) -> Vec<RepoFileResponse> {
    let projection = project_graph(&repo.policy, &repo.graph, principal);
    let git = build_virtual_git_projection(&projection);
    let mut files = git
        .blobs
        .into_iter()
        .map(|blob| {
            let scope_path =
                ScopePath::parse(&blob.path).expect("projected Git blob paths are absolute");
            RepoFileResponse {
                path: blob.path,
                oid: blob.oid,
                tracked: true,
                visibility: repo.policy.effective_visibility(&scope_path),
            }
        })
        .collect::<Vec<_>>();
    files.sort_by(|left, right| left.path.cmp(&right.path));
    files
}

fn pending_import_files(
    repo: &StoredRepository,
    principal: &Principal,
) -> Result<Vec<RepoFileResponse>, ApiError> {
    let Some(pending) = repo.pending_import.as_ref() else {
        return Ok(Vec::new());
    };
    let mut files = Vec::new();
    for file in &pending.files {
        let path = pending_scope_path(&file.path)?;
        if !repo.policy.can_read(principal, &path) {
            continue;
        }
        files.push(RepoFileResponse {
            path: path.as_str().to_string(),
            oid: file.oid.clone(),
            tracked: true,
            visibility: repo.policy.effective_visibility(&path),
        });
    }
    files.sort_by(|left, right| left.path.cmp(&right.path));
    Ok(files)
}

fn files_for_visibility_update(
    repo: &StoredRepository,
    principal: &Principal,
) -> Result<Vec<RepoFileResponse>, ApiError> {
    if repo.record.publication_state == RepoPublicationState::PendingPublish {
        pending_import_files(repo, principal)
    } else {
        Ok(projected_files(repo, principal))
    }
}

fn pending_import_changes(pending: &PendingImport) -> Result<Vec<FileChange>, ApiError> {
    pending
        .files
        .iter()
        .map(|file| {
            let content = BASE64
                .decode(file.content_base64.as_bytes())
                .map_err(ApiError::bad_request)?;
            let content = String::from_utf8(content).map_err(ApiError::bad_request)?;
            Ok(FileChange {
                path: pending_scope_path(&file.path)?,
                old_content: None,
                new_content: Some(content),
            })
        })
        .collect()
}

fn pending_scope_path(path: &str) -> Result<ScopePath, ApiError> {
    let path = if path.starts_with('/') {
        path.to_string()
    } else {
        format!("/{path}")
    };
    ScopePath::parse(path).map_err(ApiError::bad_request)
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
    fn bad_request(error: impl std::fmt::Display) -> Self {
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

    fn conflict(message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::CONFLICT,
            message: message.into(),
        }
    }

    fn unauthorized(message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::UNAUTHORIZED,
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

    fn internal_message(message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::INTERNAL_SERVER_ERROR,
            message: message.into(),
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
    use axum::{
        body::{Body, to_bytes},
        http::{Request, header::CONTENT_TYPE},
    };
    use jsonwebtoken::{EncodingKey, Header, encode};
    use scope_policy::{Principal, PrincipalKind, VisibilityRule};
    use scope_projection::SourceGraph;
    use scope_store::{
        AccountAccess, RepoMembership, RepoPublicationState, RepoRecord, StoredRepository,
        UserAccount,
    };
    use std::time::{SystemTime, UNIX_EPOCH};
    use tower::ServiceExt;

    const TEST_PAIRWISE_SUB: &str = "pairwise-owner";
    const TEST_OWNER_EMAIL: &str = "owner@example.com";
    const TEST_REPO_OWNER: &str = "owner";
    const TEST_REPO_NAME: &str = "repo";
    const TEST_REPO_ID: &str = "owner/repo";

    const TEST_PRIVATE_KEY: &str = r#"-----BEGIN PRIVATE KEY-----
MIGHAgEAMBMGByqGSM49AgEGCCqGSM49AwEHBG0wawIBAQQgj30p9gYDpHRqbshS
LyBNueRnRb9WS031zFD7yuhqn/ChRANCAAR6wR8PANHsn10BAVi085aM8LBPL3Cj
kGxvBjzgF9RjXJoldYnFk7mJ5gLANHjaaad3qTQJ8DldKJoSqkEkm5gg
-----END PRIVATE KEY-----"#;

    const TEST_JWKS: &str = r#"{
      "keys": [{
        "kty": "EC",
        "x": "esEfDwDR7J9dAQFYtPOWjPCwTy9wo5BsbwY84BfUY1w",
        "y": "miV1icWTuYnmAsA0eNppp3epNAnwOV0omhKqQSSbmCA",
        "crv": "P-256",
        "kid": "test-key",
        "use": "sig",
        "alg": "ES256"
      }]
    }"#;

    fn test_jwks() -> JwkSet {
        serde_json::from_str(TEST_JWKS).unwrap()
    }

    fn token(audience: &str, pairwise_sub: &str, email_verified: bool) -> String {
        token_for(
            audience,
            pairwise_sub,
            Some(TEST_OWNER_EMAIL.to_string()),
            email_verified,
        )
    }

    fn token_for(
        audience: &str,
        pairwise_sub: &str,
        email: Option<String>,
        email_verified: bool,
    ) -> String {
        let mut header = Header::new(Algorithm::ES256);
        header.kid = Some("test-key".to_string());
        let claims = ShooClaims {
            pairwise_sub: pairwise_sub.to_string(),
            email,
            email_verified: Some(email_verified),
        };
        let claims = serde_json::json!({
            "iss": SHOO_ISSUER,
            "aud": audience,
            "exp": unix_now() + 300,
            "pairwise_sub": claims.pairwise_sub,
            "email": claims.email,
            "email_verified": claims.email_verified,
        });

        encode(
            &header,
            &claims,
            &EncodingKey::from_ec_pem(TEST_PRIVATE_KEY.as_bytes()).unwrap(),
        )
        .unwrap()
    }

    fn token_without_origin_claims() -> String {
        let mut header = Header::new(Algorithm::ES256);
        header.kid = Some("test-key".to_string());
        let claims = serde_json::json!({
            "exp": unix_now() + 300,
            "pairwise_sub": "pairwise-owner",
            "email": TEST_OWNER_EMAIL,
            "email_verified": true,
        });

        encode(
            &header,
            &claims,
            &EncodingKey::from_ec_pem(TEST_PRIVATE_KEY.as_bytes()).unwrap(),
        )
        .unwrap()
    }

    fn unix_now() -> u64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs()
    }

    fn owner_identity(email_verified: bool) -> ShooIdentity {
        ShooIdentity {
            pairwise_sub: TEST_PAIRWISE_SUB.to_string(),
            email: Some(TEST_OWNER_EMAIL.to_string()),
            email_verified,
        }
    }

    fn test_owner_id() -> String {
        identity_user_id(&owner_identity(true))
    }

    fn test_state_with_repo() -> AppState {
        let owner_id = test_owner_id();
        let owner = UserAccount {
            id: owner_id.clone(),
            handle: TEST_REPO_OWNER.to_string(),
            email: TEST_OWNER_EMAIL.to_string(),
            email_verified: true,
            access: AccountAccess::Member,
        };
        let repo = test_repo(&owner_id);

        AppState {
            catalog: Arc::new(Mutex::new(AppCatalog {
                users: BTreeMap::from([(owner.id.clone(), owner)]),
                repositories: BTreeMap::from([(repo.record.id.clone(), repo)]),
            })),
            state_path: Arc::new(test_state_path()),
            shoo: ShooVerifier::new(
                SHOO_ISSUER,
                Some("origin:http://localhost:3000".to_string()),
                "http://127.0.0.1/.well-known/jwks.json",
            ),
        }
    }

    fn test_state_with_jwks() -> AppState {
        let state = AppState::test_state();
        cache_test_jwks(&state);
        state
    }

    fn cache_test_jwks(state: &AppState) {
        *state
            .shoo
            .jwks_cache
            .lock()
            .expect("test JWKS lock must not be poisoned") = Some(test_jwks());
    }

    fn bearer_header() -> String {
        format!(
            "Bearer {}",
            token("origin:http://localhost:3000", TEST_PAIRWISE_SUB, true)
        )
    }

    fn bearer_header_for(pairwise_sub: &str, email: &str) -> String {
        format!(
            "Bearer {}",
            token_for(
                "origin:http://localhost:3000",
                pairwise_sub,
                Some(email.to_string()),
                true,
            )
        )
    }

    async fn response_json(response: Response) -> serde_json::Value {
        let body = to_bytes(response.into_body(), 1024 * 1024).await.unwrap();
        serde_json::from_slice(&body).unwrap()
    }

    fn temp_git_repo(label: &str) -> PathBuf {
        let repo = std::env::temp_dir().join(format!(
            "scope-vcs-{label}-{}-{}",
            std::process::id(),
            unix_now()
        ));
        let _ = fs::remove_dir_all(&repo);
        fs::create_dir_all(&repo).unwrap();
        run_git(
            None,
            &["init", "-b", "main", repo.to_str().unwrap()],
            "init test repo",
        )
        .unwrap();
        repo
    }

    fn commit_all(repo: &FsPath, message: &str) {
        run_git(
            Some(repo),
            &[
                "-c",
                "user.name=Scope Test",
                "-c",
                "user.email=scope-test@example.test",
                "commit",
                "-m",
                message,
            ],
            "commit test repo",
        )
        .unwrap();
    }

    fn test_repo(owner_id: &str) -> StoredRepository {
        StoredRepository {
            record: RepoRecord {
                id: TEST_REPO_ID.to_string(),
                owner_handle: TEST_REPO_OWNER.to_string(),
                name: TEST_REPO_NAME.to_string(),
                owner_user_id: owner_id.to_string(),
                publication_state: RepoPublicationState::Published,
                default_visibility: Visibility::Public,
            },
            settings: RepoSettings::default(),
            first_push_token: None,
            pending_import: None,
            policy: Policy::new(Visibility::Public, owner_id),
            graph: SourceGraph {
                repo_id: TEST_REPO_ID.to_string(),
                commits: Vec::new(),
            },
            memberships: vec![RepoMembership {
                repo_id: TEST_REPO_ID.to_string(),
                user_id: owner_id.to_string(),
                role: RepoRole::Owner,
            }],
            invitations: Vec::new(),
        }
    }

    fn pending_import_fixture(files: Vec<(&str, &str)>) -> PendingImport {
        PendingImport {
            default_branch: "main".to_string(),
            head_oid: "1111111111111111111111111111111111111111".to_string(),
            tree_oid: "2222222222222222222222222222222222222222".to_string(),
            imported_at_unix: unix_now(),
            files: files
                .into_iter()
                .map(|(path, content)| PendingImportFile {
                    path: path.to_string(),
                    mode: "100644".to_string(),
                    oid: format!("oid-{path}"),
                    content_base64: BASE64.encode(content),
                })
                .collect(),
        }
    }

    #[test]
    fn test_state_starts_without_repositories() {
        let state = AppState::test_state();
        let error = find_repo(&state, TEST_REPO_OWNER, TEST_REPO_NAME).unwrap_err();

        assert_eq!(error.status, StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn create_repo_route_creates_user_and_lists_repo() {
        let state = test_state_with_jwks();
        let app = router(state.clone());
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/v1/repos")
                    .header(AUTHORIZATION, bearer_header())
                    .header(CONTENT_TYPE, "application/json")
                    .body(Body::from(r#"{"name":"Scope_App"}"#))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let body = response_json(response).await;
        assert_eq!(body["repo"]["id"], "owner/scope_app");
        assert_eq!(body["repo"]["owner_handle"], "owner");
        assert_eq!(body["repo"]["lifecycle_state"], "PendingFirstPush");
        assert_eq!(body["repo"]["default_visibility"], "Private");
        assert_eq!(body["repo"]["role"], "Owner");
        assert_eq!(body["setup"]["git_remote_path"], "/git/owner/scope_app");
        assert_eq!(body["setup"]["remote_name"], "scope");
        assert_eq!(body["setup"]["push_branch"], "<branch>");
        assert_eq!(body["setup"]["push_enabled"], false);
        let secret = body["setup"]["token"]["secret"].as_str().unwrap();
        assert!(secret.starts_with("scope_fp_"));
        assert_eq!(body["setup"]["token"]["status"], "Active");

        let response = app
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri("/v1/repos")
                    .header(AUTHORIZATION, bearer_header())
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let body = response_json(response).await;
        assert_eq!(body.as_array().unwrap().len(), 1);
        assert_eq!(body[0]["id"], "owner/scope_app");

        let catalog = lock_catalog(&state).unwrap();
        assert_eq!(catalog.users.len(), 1);
        assert_eq!(catalog.repositories.len(), 1);
        let repo = catalog.repositories.get("owner/scope_app").unwrap();
        let token = repo.first_push_token.as_ref().unwrap();
        assert_ne!(token.token_hash, secret);
        assert!(token.token_hash.starts_with("sha256:"));
        assert_eq!(token.owner_user_id, test_owner_id());
    }

    #[tokio::test]
    async fn setup_route_is_owner_only_and_does_not_return_secret() {
        let state = test_state_with_repo();
        cache_test_jwks(&state);
        {
            let mut catalog = lock_catalog(&state).unwrap();
            let repo = catalog.repositories.get_mut(TEST_REPO_ID).unwrap();
            let (_, token) = generate_first_push_token(&test_owner_id()).unwrap();
            repo.record.publication_state = RepoPublicationState::PendingFirstPush;
            repo.first_push_token = Some(token);
        }
        let app = router(state);

        let public_response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri("/v1/repos/owner/repo/setup")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(public_response.status(), StatusCode::UNAUTHORIZED);

        let non_owner_response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri("/v1/repos/owner/repo/setup")
                    .header(
                        AUTHORIZATION,
                        bearer_header_for("pairwise-stranger", "stranger@example.com"),
                    )
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(non_owner_response.status(), StatusCode::NOT_FOUND);

        let non_owner_regenerate_response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/v1/repos/owner/repo/setup-token")
                    .header(
                        AUTHORIZATION,
                        bearer_header_for("pairwise-stranger", "stranger@example.com"),
                    )
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(
            non_owner_regenerate_response.status(),
            StatusCode::NOT_FOUND
        );

        let owner_response = app
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri("/v1/repos/owner/repo/setup")
                    .header(AUTHORIZATION, bearer_header())
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(owner_response.status(), StatusCode::OK);
        let body = response_json(owner_response).await;
        assert_eq!(body["repo"]["id"], TEST_REPO_ID);
        assert_eq!(body["token"]["status"], "Active");
        assert!(body["token"]["secret"].is_null());
    }

    #[tokio::test]
    async fn setup_token_regeneration_rotates_hash_and_returns_new_secret() {
        let state = test_state_with_repo();
        cache_test_jwks(&state);
        let old_hash = {
            let mut catalog = lock_catalog(&state).unwrap();
            let repo = catalog.repositories.get_mut(TEST_REPO_ID).unwrap();
            let (_, token) = generate_first_push_token(&test_owner_id()).unwrap();
            let old_hash = token.token_hash.clone();
            repo.record.publication_state = RepoPublicationState::PendingFirstPush;
            repo.first_push_token = Some(token);
            old_hash
        };

        let response = router(state.clone())
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/v1/repos/owner/repo/setup-token")
                    .header(AUTHORIZATION, bearer_header())
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let body = response_json(response).await;
        let secret = body["token"]["secret"].as_str().unwrap();
        assert!(secret.starts_with("scope_fp_"));
        let catalog = lock_catalog(&state).unwrap();
        let new_hash = &catalog
            .repositories
            .get(TEST_REPO_ID)
            .unwrap()
            .first_push_token
            .as_ref()
            .unwrap()
            .token_hash;
        assert_ne!(new_hash, &old_hash);
        assert_ne!(new_hash, secret);
    }

    #[tokio::test]
    async fn list_repos_route_requires_sign_in() {
        let response = router(test_state_with_jwks())
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri("/v1/repos")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn list_repos_route_hides_pending_repo_from_reader_member() {
        let state = test_state_with_repo();
        cache_test_jwks(&state);
        let reader_identity = ShooIdentity {
            pairwise_sub: "pairwise-reader".to_string(),
            email: Some("reader@example.com".to_string()),
            email_verified: true,
        };
        let reader_id = identity_user_id(&reader_identity);
        {
            let mut catalog = lock_catalog(&state).unwrap();
            catalog.users.insert(
                reader_id.clone(),
                UserAccount {
                    id: reader_id.clone(),
                    handle: "reader".to_string(),
                    email: "reader@example.com".to_string(),
                    email_verified: true,
                    access: AccountAccess::Member,
                },
            );
            let repo = catalog.repositories.get_mut(TEST_REPO_ID).unwrap();
            repo.record.publication_state = RepoPublicationState::PendingPublish;
            repo.memberships.push(RepoMembership {
                repo_id: TEST_REPO_ID.to_string(),
                user_id: reader_id,
                role: RepoRole::Reader,
            });
        }

        let response = router(state)
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri("/v1/repos")
                    .header(
                        AUTHORIZATION,
                        bearer_header_for("pairwise-reader", "reader@example.com"),
                    )
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let body = response_json(response).await;
        assert_eq!(body.as_array().unwrap().len(), 0);
    }

    #[tokio::test]
    async fn pending_publish_repo_session_is_owner_only() {
        let state = test_state_with_repo();
        cache_test_jwks(&state);
        {
            let mut catalog = lock_catalog(&state).unwrap();
            let repo = catalog.repositories.get_mut(TEST_REPO_ID).unwrap();
            repo.record.publication_state = RepoPublicationState::PendingPublish;
        }
        let app = router(state);

        let public_response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri("/v1/repos/owner/repo/session")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(public_response.status(), StatusCode::NOT_FOUND);

        let owner_response = app
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri("/v1/repos/owner/repo/session")
                    .header(AUTHORIZATION, bearer_header())
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(owner_response.status(), StatusCode::OK);
        let body = response_json(owner_response).await;
        assert_eq!(body["principal_id"], test_owner_id());
        assert_eq!(body["capabilities"]["read"], true);
    }

    #[test]
    fn anonymous_request_uses_public_principal() {
        let state = test_state_with_repo();
        let repo = find_repo(&state, TEST_REPO_OWNER, TEST_REPO_NAME).unwrap();
        let principal = principal_for_repo(&state, &repo, None).unwrap();

        assert_eq!(principal, Principal::public());
    }

    #[test]
    fn verified_member_email_uses_repo_principal() {
        let state = test_state_with_repo();
        let repo = find_repo(&state, TEST_REPO_OWNER, TEST_REPO_NAME).unwrap();
        let identity = owner_identity(true);
        let principal = principal_for_repo(&state, &repo, Some(&identity)).unwrap();

        assert_eq!(principal.id, test_owner_id());
        assert_eq!(principal.kind, PrincipalKind::User);
    }

    #[test]
    fn unverified_email_still_uses_pairwise_user_principal() {
        let state = test_state_with_repo();
        let repo = find_repo(&state, TEST_REPO_OWNER, TEST_REPO_NAME).unwrap();
        let identity = owner_identity(false);
        let principal = principal_for_repo(&state, &repo, Some(&identity)).unwrap();

        assert_eq!(principal.id, test_owner_id());
        assert_eq!(principal.kind, PrincipalKind::User);
    }

    #[test]
    fn unreadable_repo_is_hidden_from_public_requests() {
        let state = test_state_with_repo();
        let mut repo = find_repo(&state, TEST_REPO_OWNER, TEST_REPO_NAME)
            .unwrap()
            .clone();
        repo.record.publication_state = RepoPublicationState::PendingPublish;

        let error = ensure_repo_read(&state, &repo, &Principal::public()).unwrap_err();

        assert_eq!(error.status, StatusCode::NOT_FOUND);
    }

    #[test]
    fn pending_import_review_uses_default_visibility() {
        let mut repo = test_repo(&test_owner_id());
        repo.record.publication_state = RepoPublicationState::PendingPublish;
        repo.record.default_visibility = Visibility::Private;
        repo.policy = Policy::new(Visibility::Private, repo.record.owner_user_id.clone());
        repo.pending_import = Some(pending_import_fixture(vec![
            ("README.md", "hello"),
            ("src/main.rs", "fn main() {}"),
        ]));
        let owner = Principal {
            id: repo.record.owner_user_id.clone(),
            kind: PrincipalKind::User,
        };

        let files = pending_import_files(&repo, &owner).unwrap();

        assert_eq!(files.len(), 2);
        assert!(
            files
                .iter()
                .all(|file| file.visibility == Visibility::Private)
        );
    }

    #[test]
    fn pending_visibility_toggles_apply_before_publish() {
        let mut repo = test_repo(&test_owner_id());
        repo.record.publication_state = RepoPublicationState::PendingPublish;
        repo.pending_import = Some(pending_import_fixture(vec![("README.md", "hello")]));
        let path = ScopePath::parse("/README.md").unwrap();
        repo.policy
            .add_rule(VisibilityRule::private(path.clone(), repo_owner_ids(&repo)))
            .unwrap();
        let owner = Principal {
            id: repo.record.owner_user_id.clone(),
            kind: PrincipalKind::User,
        };

        let private_files = files_for_visibility_update(&repo, &owner).unwrap();
        assert_eq!(private_files[0].visibility, Visibility::Private);

        repo.policy.add_rule(VisibilityRule::public(path)).unwrap();
        let public_files = files_for_visibility_update(&repo, &owner).unwrap();
        assert_eq!(public_files[0].visibility, Visibility::Public);
    }

    #[test]
    fn zero_file_publish_promotes_pending_import() {
        let mut repo = test_repo(&test_owner_id());
        repo.record.publication_state = RepoPublicationState::PendingPublish;
        repo.pending_import = Some(pending_import_fixture(Vec::new()));

        promote_pending_import(&mut repo).unwrap();

        assert_eq!(
            repo.record.publication_state,
            RepoPublicationState::Published
        );
        assert!(repo.pending_import.is_none());
        assert_eq!(repo.graph.commits.len(), 1);
        assert!(repo.graph.commits[0].changes.is_empty());
    }

    #[test]
    fn publish_is_one_time() {
        let mut repo = test_repo(&test_owner_id());
        repo.record.publication_state = RepoPublicationState::PendingPublish;
        repo.pending_import = Some(pending_import_fixture(vec![("README.md", "hello")]));

        promote_pending_import(&mut repo).unwrap();
        let error = promote_pending_import(&mut repo).unwrap_err();

        assert_eq!(error.status, StatusCode::BAD_REQUEST);
    }

    #[test]
    fn bearer_token_ignores_removed_trusted_identity_headers() {
        let mut headers = HeaderMap::new();
        headers.insert("x-scope-user-email", TEST_OWNER_EMAIL.parse().unwrap());
        headers.insert("x-scope-user-email-verified", "true".parse().unwrap());

        assert_eq!(bearer_token(&headers).unwrap(), None);
    }

    #[test]
    fn bearer_token_rejects_non_bearer_authorization() {
        let mut headers = HeaderMap::new();
        headers.insert(AUTHORIZATION, "Basic abc".parse().unwrap());

        let error = bearer_token(&headers).unwrap_err();

        assert_eq!(error.status, StatusCode::UNAUTHORIZED);
    }

    #[test]
    fn first_push_token_accepts_bearer_and_basic_password() {
        let mut headers = HeaderMap::new();
        headers.insert(AUTHORIZATION, "Bearer scope_fp_secret".parse().unwrap());
        assert_eq!(
            first_push_token_from_headers(&headers).unwrap(),
            "scope_fp_secret"
        );

        let encoded = BASE64.encode("scope:scope_fp_secret");
        headers.insert(AUTHORIZATION, format!("Basic {encoded}").parse().unwrap());
        assert_eq!(
            first_push_token_from_headers(&headers).unwrap(),
            "scope_fp_secret"
        );
    }

    #[test]
    fn pending_import_marks_token_used_after_durable_state_update() {
        let state = test_state_with_repo();
        let secret = "scope_fp_test";
        {
            let mut catalog = lock_catalog(&state).unwrap();
            let repo = catalog.repositories.get_mut(TEST_REPO_ID).unwrap();
            repo.record.publication_state = RepoPublicationState::PendingFirstPush;
            repo.first_push_token = Some(FirstPushToken {
                token_hash: first_push_token_hash(secret),
                owner_user_id: repo.record.owner_user_id.clone(),
                created_at_unix: unix_now(),
                expires_at_unix: unix_now() + FIRST_PUSH_TOKEN_TTL_SECS,
                used_at_unix: None,
            });
            repo.pending_import = None;
        }

        let import = PendingImport {
            default_branch: "main".to_string(),
            head_oid: "1111111111111111111111111111111111111111".to_string(),
            tree_oid: "2222222222222222222222222222222222222222".to_string(),
            imported_at_unix: unix_now(),
            files: Vec::new(),
        };

        persist_pending_import(&state, TEST_REPO_OWNER, TEST_REPO_NAME, secret, import).unwrap();

        let repo = find_repo(&state, TEST_REPO_OWNER, TEST_REPO_NAME).unwrap();
        assert_eq!(
            repo.record.publication_state,
            RepoPublicationState::PendingPublish
        );
        assert_eq!(repo.pending_import.as_ref().unwrap().default_branch, "main");
        assert!(repo.first_push_token.unwrap().used_at_unix.is_some());

        let error =
            authorize_first_push(&state, TEST_REPO_OWNER, TEST_REPO_NAME, secret).unwrap_err();
        assert_eq!(error.status, StatusCode::CONFLICT);
    }

    #[test]
    fn pushed_tree_rejects_gitlinks_instead_of_dropping_them() {
        let repo = temp_git_repo("gitlink-test");
        fs::write(repo.join("README.md"), "hello").unwrap();
        run_git(Some(&repo), &["add", "README.md"], "add readme").unwrap();
        commit_all(&repo, "initial");
        let commit = git_stdout_text(&repo, &["rev-parse", "HEAD"], "read head")
            .unwrap()
            .trim()
            .to_string();
        run_git(
            Some(&repo),
            &[
                "update-index",
                "--add",
                "--cacheinfo",
                &format!("160000,{commit},vendor/submodule"),
            ],
            "add gitlink",
        )
        .unwrap();
        commit_all(&repo, "add gitlink");

        let error = git_tree_files(&repo, "HEAD").unwrap_err();

        assert_eq!(error.status, StatusCode::BAD_REQUEST);
        assert!(error.message.contains("unsupported Git tree entry"));
        let _ = fs::remove_dir_all(&repo);
    }

    #[test]
    fn pushed_tree_rejects_non_utf8_blobs_before_pending_import() {
        let repo = temp_git_repo("binary-test");
        fs::write(repo.join("image.bin"), [0xff, 0x00, 0x61]).unwrap();
        run_git(Some(&repo), &["add", "image.bin"], "add binary").unwrap();
        commit_all(&repo, "binary");

        let error = git_tree_files(&repo, "HEAD").unwrap_err();

        assert_eq!(error.status, StatusCode::BAD_REQUEST);
        assert!(error.message.contains("valid UTF-8 text"));
        let _ = fs::remove_dir_all(&repo);
    }

    #[test]
    fn pushed_tree_rejects_modes_that_projection_cannot_preserve() {
        let repo = temp_git_repo("mode-test");
        fs::write(repo.join("script.sh"), "#!/bin/sh\necho hi\n").unwrap();
        run_git(Some(&repo), &["add", "script.sh"], "add script").unwrap();
        run_git(
            Some(&repo),
            &["update-index", "--chmod=+x", "script.sh"],
            "make script executable",
        )
        .unwrap();
        commit_all(&repo, "executable");

        let error = git_tree_files(&repo, "HEAD").unwrap_err();

        assert_eq!(error.status, StatusCode::BAD_REQUEST);
        assert!(error.message.contains("unsupported Git file mode"));
        let _ = fs::remove_dir_all(&repo);
    }

    #[test]
    fn pushed_tree_rejects_paths_scope_would_normalize_or_git_cannot_serve() {
        validate_pushed_file_path("docs/read me.md").unwrap();
        for path in [
            "README.md ",
            "dir\\file.txt",
            "line\nbreak.txt",
            "./README.md",
            "docs/../README.md",
        ] {
            let error = validate_pushed_file_path(path).unwrap_err();
            assert_eq!(error.status, StatusCode::BAD_REQUEST);
        }
    }

    #[test]
    fn shoo_token_verifies_issuer_audience_signature_expiration_and_pairwise_sub() {
        let jwt = token("origin:http://localhost:3000", TEST_PAIRWISE_SUB, true);
        let identity = verify_shoo_token(
            &jwt,
            &test_jwks(),
            SHOO_ISSUER,
            "origin:http://localhost:3000",
        )
        .unwrap();

        assert_eq!(identity.pairwise_sub, TEST_PAIRWISE_SUB);
        assert_eq!(identity.email.as_deref(), Some(TEST_OWNER_EMAIL));
        assert!(identity.email_verified);
    }

    #[test]
    fn shoo_token_rejects_wrong_audience() {
        let jwt = token("origin:https://other.example", TEST_PAIRWISE_SUB, true);
        let error = verify_shoo_token(
            &jwt,
            &test_jwks(),
            SHOO_ISSUER,
            "origin:http://localhost:3000",
        )
        .unwrap_err();

        assert_eq!(error.status, StatusCode::UNAUTHORIZED);
    }

    #[test]
    fn shoo_token_requires_issuer_and_audience_claims() {
        let jwt = token_without_origin_claims();
        let error = verify_shoo_token(
            &jwt,
            &test_jwks(),
            SHOO_ISSUER,
            "origin:http://localhost:3000",
        )
        .unwrap_err();

        assert_eq!(error.status, StatusCode::UNAUTHORIZED);
    }

    #[test]
    fn shoo_token_requires_pairwise_sub() {
        let jwt = token("origin:http://localhost:3000", "", true);
        let error = verify_shoo_token(
            &jwt,
            &test_jwks(),
            SHOO_ISSUER,
            "origin:http://localhost:3000",
        )
        .unwrap_err();

        assert_eq!(error.status, StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn shoo_verifier_requires_configured_audience() {
        let verifier =
            ShooVerifier::new(SHOO_ISSUER, None, "http://127.0.0.1/.well-known/jwks.json");
        let jwt = token("origin:http://localhost:3000", TEST_PAIRWISE_SUB, true);
        let error = verifier.verify(&jwt).await.unwrap_err();

        assert_eq!(error.status, StatusCode::SERVICE_UNAVAILABLE);
    }
}
