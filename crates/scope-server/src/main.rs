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
use scope_policy::{Policy, Principal, ScopePath, Visibility, VisibilityRule};
use scope_projection::{
    AuthorVisibility, FileChange, LogicalCommit, MixedCommitPolicy, Projection, project_graph,
};
use scope_store::{
    AppCatalog, BOOTSTRAP_REPO_ID, RepoRole, RepoSettings, StoredRepository, VerifiedEmail,
    app_catalog, bootstrap_path_is_public, build_repository_snapshot_changes,
};
use serde::{Deserialize, Serialize};
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
const MAX_OWNER_FILE_PATHS: usize = 5_000;

#[derive(Clone)]
struct AppState {
    catalog: Arc<Mutex<AppCatalog>>,
    repo_root: Arc<PathBuf>,
    state_path: Arc<PathBuf>,
    shoo: ShooVerifier,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
struct PersistedState {
    repositories: BTreeMap<String, PersistedRepositoryState>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct PersistedRepositoryState {
    settings: RepoSettings,
    policy: Policy,
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
    let role = role_for_principal(&state, &repo, &principal)?;

    if role == Some(RepoRole::Owner) {
        return match git_files(&state.repo_root, &repo, &principal, role) {
            Ok(files) => Ok(Json(files)),
            Err(error) if error.status == StatusCode::SERVICE_UNAVAILABLE => {
                tracing::warn!(error = %error.message, "falling back to projected files");
                Ok(Json(projected_files(&repo, &principal)))
            }
            Err(error) => Err(error),
        };
    }

    Ok(Json(projected_files(&repo, &principal)))
}

async fn update_file_visibility(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((owner, repo_name)): Path<(String, String)>,
    Json(input): Json<UpdateFileVisibilityRequest>,
) -> Result<Json<RepoFileResponse>, ApiError> {
    let identity = http_identity(&state, &headers).await?;
    let verified_email = identity.as_ref().and_then(ShooIdentity::verified_email);
    let path = ScopePath::parse(&input.path).map_err(ApiError::bad_request)?;
    let repo_id = scope_store::repo_id(&owner, &repo_name);

    let repo = find_repo(&state, &owner, &repo_name)?;
    let principal = principal_for_repo(&state, &repo, identity.as_ref())?;
    ensure_repo_read(&state, &repo, &principal)?;
    if role_for_principal(&state, &repo, &principal)? != Some(RepoRole::Owner) {
        return Err(ApiError::forbidden("owner role required"));
    }

    let (owner_files, owner_files_from_git) =
        match git_files(&state.repo_root, &repo, &principal, Some(RepoRole::Owner)) {
            Ok(files) => (files, true),
            Err(error) if error.status == StatusCode::SERVICE_UNAVAILABLE => {
                tracing::warn!(error = %error.message, "falling back to projected files");
                (projected_files(&repo, &principal), false)
            }
            Err(error) => return Err(error),
        };
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
        if owner_files_from_git {
            hydrate_catalog_from_git(&mut staged, state.repo_root.as_ref())?;
        }
        let repo = staged
            .repositories
            .get(&repo_id)
            .ok_or_else(|| ApiError::not_found(format!("repo {owner}/{repo_name} not found")))?;
        let principal = staged.principal_for_repo(repo, verified_email.as_ref());
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
    let updated_files = match git_files(
        &state.repo_root,
        &updated,
        &principal,
        Some(RepoRole::Owner),
    ) {
        Ok(files) => files,
        Err(error) if error.status == StatusCode::SERVICE_UNAVAILABLE => {
            tracing::warn!(error = %error.message, "falling back to projected files");
            projected_files(&updated, &principal)
        }
        Err(error) => return Err(error),
    };
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
    let verified_email = identity.as_ref().and_then(ShooIdentity::verified_email);
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
    let principal = staged.principal_for_repo(repo, verified_email.as_ref());

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
        hydrate_catalog_from_sources(&mut catalog, &repo_root)?;

        Ok(Self {
            catalog: Arc::new(Mutex::new(catalog)),
            repo_root: Arc::new(repo_root),
            state_path: Arc::new(state_path),
            shoo: ShooVerifier::from_env(),
        })
    }

    #[cfg(test)]
    fn test_state() -> Self {
        Self {
            catalog: Arc::new(Mutex::new(app_catalog())),
            repo_root: Arc::new(git_repo_root()),
            state_path: Arc::new(std::env::temp_dir().join("scope-vcs-test-state.json")),
            shoo: ShooVerifier::new(
                SHOO_ISSUER,
                Some("origin:http://localhost:3000".to_string()),
                "http://127.0.0.1/.well-known/jwks.json",
            ),
        }
    }
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
    for (repo_id, persisted) in &state.repositories {
        let Some(repo) = catalog.repositories.get_mut(repo_id) else {
            continue;
        };

        repo.settings = persisted.settings;
        repo.policy = persisted.policy.clone();
    }
}

fn persisted_state_from_catalog(catalog: &AppCatalog) -> PersistedState {
    PersistedState {
        repositories: catalog
            .repositories
            .iter()
            .map(|(repo_id, repo)| {
                (
                    repo_id.clone(),
                    PersistedRepositoryState {
                        settings: repo.settings,
                        policy: repo.policy.clone(),
                    },
                )
            })
            .collect(),
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

impl ShooIdentity {
    fn verified_email(&self) -> Option<VerifiedEmail> {
        self.email
            .as_ref()
            .map(|email| VerifiedEmail::new(email, self.email_verified))
    }
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
    let verified_email = identity.and_then(ShooIdentity::verified_email);
    Ok(lock_catalog(state)?.principal_for_repo(repo, verified_email.as_ref()))
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

fn hydrate_catalog_from_sources(
    catalog: &mut AppCatalog,
    repo_root: &FsPath,
) -> anyhow::Result<()> {
    match git_tracked_file_changes(repo_root) {
        Ok(changes) if !changes.is_empty() => {
            install_repository_changes(
                catalog,
                changes,
                "rv_git_worktree_head".to_string(),
                "Import tracked repository files".to_string(),
            );
            Ok(())
        }
        Ok(_) => {
            hydrate_catalog_from_build_snapshot(catalog, "Git index did not contain tracked files")
        }
        Err(error) => hydrate_catalog_from_build_snapshot(catalog, &error.message),
    }
}

fn hydrate_catalog_from_git(catalog: &mut AppCatalog, repo_root: &FsPath) -> Result<(), ApiError> {
    let changes = git_tracked_file_changes(repo_root)?;
    if changes.is_empty() {
        return Err(ApiError::service_unavailable(
            "Git index did not contain tracked files",
        ));
    }

    install_repository_changes(
        catalog,
        changes,
        "rv_git_worktree_head".to_string(),
        "Import tracked repository files".to_string(),
    );
    Ok(())
}

fn hydrate_catalog_from_build_snapshot(
    catalog: &mut AppCatalog,
    reason: &str,
) -> anyhow::Result<()> {
    let changes = build_repository_snapshot_changes();
    if changes.is_empty() {
        anyhow::bail!(
            "failed to hydrate repository from runtime Git ({reason}) and build snapshot is empty"
        );
    }

    tracing::warn!(
        reason = %reason,
        file_count = changes.len(),
        "using compiled repository snapshot because runtime Git is unavailable"
    );
    install_repository_changes(
        catalog,
        changes,
        "rv_build_snapshot".to_string(),
        "Import tracked repository files".to_string(),
    );
    Ok(())
}

fn install_repository_changes(
    catalog: &mut AppCatalog,
    changes: Vec<FileChange>,
    id: String,
    message: String,
) {
    let Some(repo) = catalog.repositories.get_mut(BOOTSTRAP_REPO_ID) else {
        return;
    };

    let owner_ids = repo_owner_ids(repo);
    for change in &changes {
        if repo.policy.effective_rule(&change.path).is_some()
            || bootstrap_path_is_public(&change.path)
        {
            continue;
        }

        if let Err(error) = repo.policy.add_rule(VisibilityRule::private(
            change.path.clone(),
            owner_ids.clone(),
        )) {
            tracing::warn!(%error, path = change.path.as_str(), "failed to privatize hydrated path");
        }
    }

    repo.graph.commits = vec![LogicalCommit {
        id,
        parent_ids: Vec::new(),
        author_id: repo.record.owner_user_id.clone(),
        author_visibility: AuthorVisibility::Visible,
        message,
        mixed_policy: MixedCommitPolicy::SyntheticPublicCommit,
        changes,
    }];
}

fn git_tracked_file_changes(repo_root: &FsPath) -> Result<Vec<FileChange>, ApiError> {
    let mut changes = Vec::new();

    for (path, oid) in git_tracked_file_entries(repo_root)? {
        if is_internal_scope_path(&path) {
            continue;
        }
        let Some(content) = git_blob_content(repo_root, &oid, &path)? else {
            continue;
        };
        changes.push(FileChange {
            path: ScopePath::parse(format!("/{path}")).map_err(ApiError::bad_request)?,
            old_content: None,
            new_content: Some(content),
        });
    }

    Ok(changes)
}

fn git_tracked_file_entries(repo_root: &FsPath) -> Result<Vec<(String, String)>, ApiError> {
    let output = Command::new("git")
        .arg("-C")
        .arg(repo_root)
        .args(["ls-files", "-z", "-s"])
        .output()
        .map_err(|error| {
            ApiError::service_unavailable(format!("failed to read Git index: {error}"))
        })?;

    if !output.status.success() {
        return Err(ApiError::service_unavailable(
            String::from_utf8_lossy(&output.stderr).trim().to_string(),
        ));
    }

    let mut entries = Vec::new();
    for raw in output.stdout.split(|byte| *byte == 0) {
        if raw.is_empty() {
            continue;
        }
        let entry = std::str::from_utf8(raw).map_err(ApiError::bad_request)?;
        let Some((metadata, path)) = entry.split_once('\t') else {
            return Err(ApiError::internal_message(format!(
                "unexpected git ls-files entry: {entry}"
            )));
        };
        let oid = metadata
            .split_whitespace()
            .nth(1)
            .ok_or_else(|| ApiError::internal_message("git entry is missing an object id"))?;
        entries.push((path.replace('\\', "/"), oid.to_string()));
    }

    Ok(entries)
}

fn git_blob_content(repo_root: &FsPath, oid: &str, path: &str) -> Result<Option<String>, ApiError> {
    let output = Command::new("git")
        .arg("-C")
        .arg(repo_root)
        .args(["cat-file", "blob", oid])
        .output()
        .map_err(|error| {
            ApiError::service_unavailable(format!("failed to read Git blob {oid}: {error}"))
        })?;

    if !output.status.success() {
        return Err(ApiError::service_unavailable(
            String::from_utf8_lossy(&output.stderr).trim().to_string(),
        ));
    }

    match String::from_utf8(output.stdout) {
        Ok(content) => Ok(Some(content)),
        Err(_) => {
            tracing::warn!(path, "skipping non-UTF-8 file during repository hydration");
            Ok(None)
        }
    }
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

fn is_internal_scope_path(path: &str) -> bool {
    path == ".scope" || path.starts_with(".scope/")
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

fn git_files(
    repo_root: &FsPath,
    repo: &StoredRepository,
    principal: &Principal,
    role: Option<RepoRole>,
) -> Result<Vec<RepoFileResponse>, ApiError> {
    let include_worktree_files = role == Some(RepoRole::Owner);
    let entries = git_file_paths(
        repo_root,
        include_worktree_files,
        include_worktree_files && repo.settings.include_ignored_files,
    )?;
    let worktree_paths = entries
        .iter()
        .filter(|entry| entry.oid.is_none())
        .map(|entry| entry.path.clone())
        .collect::<Vec<_>>();
    let worktree_oid_by_path = git_blob_oids(repo_root, &worktree_paths)?;
    let mut files = Vec::with_capacity(entries.len());

    for entry in entries {
        let scope_path =
            ScopePath::parse(format!("/{}", entry.path)).map_err(ApiError::bad_request)?;

        if !repo.policy.can_read(principal, &scope_path) {
            continue;
        }

        let oid = entry
            .oid
            .as_ref()
            .or_else(|| worktree_oid_by_path.get(&entry.path))
            .ok_or_else(|| {
                ApiError::internal_message(format!("missing Git object for {}", entry.path))
            })?;

        files.push(RepoFileResponse {
            path: scope_path.as_str().to_string(),
            oid: oid.clone(),
            tracked: entry.tracked,
            visibility: if entry.tracked && graph_has_file(repo, &scope_path) {
                repo.policy.effective_visibility(&scope_path)
            } else {
                Visibility::Private
            },
        });
    }

    files.sort_by(|left, right| left.path.cmp(&right.path));
    Ok(files)
}

#[derive(Debug)]
struct GitFileEntry {
    path: String,
    oid: Option<String>,
    tracked: bool,
}

fn git_file_paths(
    repo_root: &FsPath,
    include_worktree_files: bool,
    include_ignored: bool,
) -> Result<Vec<GitFileEntry>, ApiError> {
    let mut path_status = git_tracked_file_entries(repo_root)?
        .into_iter()
        .map(|(path, oid)| {
            (
                path.clone(),
                GitFileEntry {
                    path,
                    oid: Some(oid),
                    tracked: true,
                },
            )
        })
        .collect::<BTreeMap<_, _>>();

    if include_worktree_files {
        for path in git_ls_files(
            repo_root,
            &["ls-files", "-z", "--others", "--exclude-standard"],
        )? {
            path_status.entry(path.clone()).or_insert(GitFileEntry {
                path,
                oid: None,
                tracked: false,
            });
        }
    }

    if include_ignored {
        for path in git_ls_files(
            repo_root,
            &[
                "ls-files",
                "-z",
                "--others",
                "--ignored",
                "--exclude-standard",
            ],
        )? {
            path_status.entry(path.clone()).or_insert(GitFileEntry {
                path,
                oid: None,
                tracked: false,
            });
        }
    }

    let entries = path_status
        .into_iter()
        .map(|(_, entry)| entry)
        .filter(|entry| {
            !is_internal_scope_path(&entry.path)
                && (entry.tracked || repo_root.join(&entry.path).is_file())
        })
        .collect::<Vec<_>>();
    if entries.len() > MAX_OWNER_FILE_PATHS {
        return Err(ApiError::bad_request(format!(
            "file list has {} paths; narrow ignored files before listing more than {}",
            entries.len(),
            MAX_OWNER_FILE_PATHS
        )));
    }
    Ok(entries)
}

fn git_ls_files(repo_root: &FsPath, args: &[&str]) -> Result<Vec<String>, ApiError> {
    let output = Command::new("git")
        .arg("-C")
        .arg(repo_root)
        .args(args)
        .output()
        .map_err(|error| {
            ApiError::service_unavailable(format!("failed to read Git index: {error}"))
        })?;

    if !output.status.success() {
        return Err(ApiError::service_unavailable(
            String::from_utf8_lossy(&output.stderr).trim().to_string(),
        ));
    }

    let mut paths = Vec::new();
    for raw in output.stdout.split(|byte| *byte == 0) {
        if raw.is_empty() {
            continue;
        }
        paths.push(
            std::str::from_utf8(raw)
                .map_err(ApiError::bad_request)?
                .replace('\\', "/"),
        );
    }

    Ok(paths)
}

fn git_blob_oids(
    repo_root: &FsPath,
    paths: &[String],
) -> Result<BTreeMap<String, String>, ApiError> {
    if paths.is_empty() {
        return Ok(BTreeMap::new());
    }

    let mut oid_by_path = BTreeMap::new();
    for chunk in paths.chunks(64) {
        oid_by_path.extend(git_blob_oids_chunk(repo_root, chunk)?);
    }

    Ok(oid_by_path)
}

fn git_blob_oids_chunk(
    repo_root: &FsPath,
    paths: &[String],
) -> Result<BTreeMap<String, String>, ApiError> {
    let mut child = Command::new("git")
        .arg("-C")
        .arg(repo_root)
        .args(["hash-object", "--stdin-paths"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| {
            ApiError::service_unavailable(format!("failed to hash Git files: {error}"))
        })?;

    {
        let stdin = child
            .stdin
            .as_mut()
            .ok_or_else(|| ApiError::internal_message("failed to open git hash-object stdin"))?;
        for path in paths {
            stdin
                .write_all(path.as_bytes())
                .map_err(ApiError::internal)?;
            stdin.write_all(b"\n").map_err(ApiError::internal)?;
        }
    }

    let output = child.wait_with_output().map_err(ApiError::internal)?;

    if !output.status.success() {
        return Err(ApiError::service_unavailable(
            String::from_utf8_lossy(&output.stderr).trim().to_string(),
        ));
    }

    let oids = String::from_utf8(output.stdout).map_err(ApiError::bad_request)?;
    let oids = oids
        .lines()
        .map(str::trim)
        .filter(|oid| !oid.is_empty())
        .map(str::to_string)
        .collect::<Vec<_>>();

    if oids.len() != paths.len() {
        return Err(ApiError::internal_message(format!(
            "git hash-object returned {} objects for {} paths",
            oids.len(),
            paths.len()
        )));
    }

    Ok(paths.iter().cloned().zip(oids).collect())
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
    use jsonwebtoken::{EncodingKey, Header, encode};
    use scope_policy::{Principal, PrincipalKind};
    use scope_store::{
        BOOTSTRAP_OWNER_EMAIL, BOOTSTRAP_OWNER_USER_ID, BOOTSTRAP_REPO_NAME, BOOTSTRAP_REPO_OWNER,
    };
    use std::time::{SystemTime, UNIX_EPOCH};

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
        let mut header = Header::new(Algorithm::ES256);
        header.kid = Some("test-key".to_string());
        let claims = ShooClaims {
            pairwise_sub: pairwise_sub.to_string(),
            email: Some(BOOTSTRAP_OWNER_EMAIL.to_string()),
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
            "email": BOOTSTRAP_OWNER_EMAIL,
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

    #[test]
    fn anonymous_request_uses_public_principal() {
        let state = AppState::test_state();
        let repo = find_repo(&state, BOOTSTRAP_REPO_OWNER, BOOTSTRAP_REPO_NAME).unwrap();
        let principal = principal_for_repo(&state, &repo, None).unwrap();

        assert_eq!(principal, Principal::public());
    }

    #[test]
    fn verified_bootstrap_email_uses_owner_principal() {
        let state = AppState::test_state();
        let repo = find_repo(&state, BOOTSTRAP_REPO_OWNER, BOOTSTRAP_REPO_NAME).unwrap();
        let identity = ShooIdentity {
            pairwise_sub: "pairwise-owner".to_string(),
            email: Some(BOOTSTRAP_OWNER_EMAIL.to_string()),
            email_verified: true,
        };
        let principal = principal_for_repo(&state, &repo, Some(&identity)).unwrap();

        assert_eq!(principal.id, BOOTSTRAP_OWNER_USER_ID);
        assert_eq!(principal.kind, PrincipalKind::User);
    }

    #[test]
    fn unverified_bootstrap_email_uses_public_principal() {
        let state = AppState::test_state();
        let repo = find_repo(&state, BOOTSTRAP_REPO_OWNER, BOOTSTRAP_REPO_NAME).unwrap();
        let identity = ShooIdentity {
            pairwise_sub: "pairwise-owner".to_string(),
            email: Some(BOOTSTRAP_OWNER_EMAIL.to_string()),
            email_verified: false,
        };
        let principal = principal_for_repo(&state, &repo, Some(&identity)).unwrap();

        assert_eq!(principal, Principal::public());
    }

    #[test]
    fn unreadable_repo_is_hidden_from_public_requests() {
        let state = AppState::test_state();
        let mut repo = find_repo(&state, BOOTSTRAP_REPO_OWNER, BOOTSTRAP_REPO_NAME)
            .unwrap()
            .clone();
        repo.record.publication_state = scope_store::RepoPublicationState::Unpublished;

        let error = ensure_repo_read(&state, &repo, &Principal::public()).unwrap_err();

        assert_eq!(error.status, StatusCode::NOT_FOUND);
    }

    #[test]
    fn bearer_token_ignores_removed_trusted_identity_headers() {
        let mut headers = HeaderMap::new();
        headers.insert("x-scope-user-email", BOOTSTRAP_OWNER_EMAIL.parse().unwrap());
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
        let jwt = token("origin:http://localhost:3000", "pairwise-owner", true);
        let identity = verify_shoo_token(
            &jwt,
            &test_jwks(),
            SHOO_ISSUER,
            "origin:http://localhost:3000",
        )
        .unwrap();

        assert_eq!(identity.pairwise_sub, "pairwise-owner");
        assert_eq!(identity.email.as_deref(), Some(BOOTSTRAP_OWNER_EMAIL));
        assert!(identity.email_verified);
    }

    #[test]
    fn shoo_token_rejects_wrong_audience() {
        let jwt = token("origin:https://other.example", "pairwise-owner", true);
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
        let jwt = token("origin:http://localhost:3000", "pairwise-owner", true);
        let error = verifier.verify(&jwt).await.unwrap_err();

        assert_eq!(error.status, StatusCode::SERVICE_UNAVAILABLE);
    }
}
