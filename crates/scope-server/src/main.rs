use anyhow::Context;
use axum::{
    Json, Router,
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode, header::AUTHORIZATION},
    response::{IntoResponse, Response},
    routing::{get, patch},
};
use jsonwebtoken::{
    Algorithm, DecodingKey, Validation, decode, decode_header,
    jwk::{Jwk, JwkSet},
};
use scope_git::{VirtualGitProjection, build_virtual_git_projection};
use scope_policy::{Principal, ScopePath, Visibility, VisibilityRule};
use scope_projection::{Projection, project_graph};
use scope_store::{
    AccountAccess, AppCatalog, CatalogError, RepoPublicationState, RepoRole, RepoSettings,
    StoredRepository, UserAccount, app_catalog,
};
use serde::{Deserialize, Serialize};
use sha1::{Digest, Sha1};
use std::{
    collections::BTreeMap,
    fs,
    io::Write,
    net::SocketAddr,
    path::{Path as FsPath, PathBuf},
    process::Command,
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
struct RepoFileResponse {
    path: String,
    oid: String,
    tracked: bool,
    visibility: Visibility,
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
        .route("/v1/repos/{owner}/{repo}/session", get(get_session))
        .route("/v1/repos/{owner}/{repo}/files", get(get_files))
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
) -> Result<Json<RepoSummaryResponse>, ApiError> {
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
        let repo = staged
            .repositories
            .get(&repo_id)
            .expect("created repository must exist");
        let summary = repo_summary(&staged, repo, &user.id).ok_or_else(|| {
            ApiError::internal_message("created repository is missing owner role")
        })?;

        persist_catalog(&state, &staged)?;
        *catalog = staged;
        summary
    };

    Ok(Json(created))
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

    let owner_files = projected_files(&repo, &principal);
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

        if input.visibility == Visibility::Public && !graph_has_file(repo, &path) {
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
    let updated_files = projected_files(&updated, &principal);
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
    use scope_policy::{Policy, Principal, PrincipalKind};
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
        assert_eq!(body["id"], "owner/scope_app");
        assert_eq!(body["owner_handle"], "owner");
        assert_eq!(body["lifecycle_state"], "PendingFirstPush");
        assert_eq!(body["default_visibility"], "Private");
        assert_eq!(body["role"], "Owner");

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
