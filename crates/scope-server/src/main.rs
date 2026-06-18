use anyhow::Context;
use axum::{
    Json, Router,
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode, header::AUTHORIZATION},
    response::{IntoResponse, Response},
    routing::{get, post},
};
use jsonwebtoken::{
    Algorithm, DecodingKey, Validation, decode, decode_header,
    jwk::{Jwk, JwkSet},
};
use scope_crypto::{ManifestMixedPolicy, PushManifest, SignedPushManifest, sign_manifest};
use scope_git::{VirtualGitProjection, build_virtual_git_projection};
use scope_policy::{Principal, ScopePath};
use scope_projection::{Projection, project_graph};
use scope_store::{AppCatalog, RepoRole, StoredRepository, VerifiedEmail, app_catalog};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    net::SocketAddr,
    sync::{Arc, Mutex},
    time::Duration,
};
use tower_http::{cors::CorsLayer, trace::TraceLayer};
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

const MANIFEST_SIGNING_SECRET_ENV: &str = "SCOPE_MANIFEST_SIGNING_SECRET";
const SCOPE_APP_ORIGIN_ENV: &str = "SCOPE_APP_ORIGIN";
const SHOO_JWKS_URL: &str = "https://shoo.dev/.well-known/jwks.json";
const SHOO_ISSUER: &str = "https://shoo.dev";
const LOCAL_APP_ORIGIN: &str = "http://localhost:3000";

#[derive(Clone)]
struct AppState {
    catalog: Arc<AppCatalog>,
    shoo: ShooVerifier,
    manifest_signing_secret: Option<Arc<[u8]>>,
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
        .route("/v1/repos/{owner}/{repo}/session", get(get_session))
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
    let identity = http_identity(&state, &headers).await?;
    let principal = principal_for_repo(&state, repo, identity.as_ref());
    Ok(Json(project_graph(&repo.policy, &repo.graph, &principal)))
}

async fn get_git_projection(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((owner, repo_name)): Path<(String, String)>,
) -> Result<Json<VirtualGitProjection>, ApiError> {
    let repo = find_repo(&state, &owner, &repo_name)?;
    let identity = http_identity(&state, &headers).await?;
    let principal = principal_for_repo(&state, repo, identity.as_ref());
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
    let principal = principal_for_repo(&state, repo, identity.as_ref());
    let root = ScopePath::root();

    Ok(Json(SessionResponse {
        identity: identity.as_ref().map(SessionIdentity::from),
        repo: SessionRepo {
            id: repo.record.id.clone(),
            role: repo_role(repo, &principal),
        },
        capabilities: SessionCapabilities {
            read: repo.policy.can_read(&principal, &root),
            write: repo.policy.can_write(&principal, &root),
        },
        principal_id: principal.id,
    }))
}

async fn create_manifest(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((owner, repo_name)): Path<(String, String)>,
    Json(input): Json<CreateManifestRequest>,
) -> Result<Json<CreateManifestResponse>, ApiError> {
    let repo = find_repo(&state, &owner, &repo_name)?;
    let identity = http_identity(&state, &headers).await?;
    let principal = principal_for_repo(&state, repo, identity.as_ref());

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
            shoo: ShooVerifier::from_env(),
            manifest_signing_secret: non_empty_env(MANIFEST_SIGNING_SECRET_ENV)
                .map(|secret| Arc::<[u8]>::from(secret.into_bytes())),
        }
    }

    #[cfg(test)]
    fn test_state(manifest_signing_secret: Option<&str>) -> Self {
        Self {
            catalog: Arc::new(app_catalog()),
            shoo: ShooVerifier::new(
                SHOO_ISSUER,
                Some("origin:http://localhost:3000".to_string()),
                "http://127.0.0.1/.well-known/jwks.json",
            ),
            manifest_signing_secret: manifest_signing_secret
                .map(|secret| Arc::<[u8]>::from(secret.as_bytes())),
        }
    }
}

fn non_empty_env(name: &str) -> Option<String> {
    std::env::var(name).ok().filter(|value| !value.is_empty())
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
) -> Principal {
    let verified_email = identity.and_then(ShooIdentity::verified_email);
    state
        .catalog
        .principal_for_repo(repo, verified_email.as_ref())
}

fn repo_role(repo: &StoredRepository, principal: &Principal) -> Option<RepoRole> {
    repo.memberships
        .iter()
        .find(|membership| membership.user_id == principal.id)
        .map(|membership| membership.role)
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
        let state = AppState::test_state(None);
        let repo = find_repo(&state, BOOTSTRAP_REPO_OWNER, BOOTSTRAP_REPO_NAME).unwrap();
        let principal = principal_for_repo(&state, repo, None);

        assert_eq!(principal, Principal::public());
    }

    #[test]
    fn verified_bootstrap_email_uses_owner_principal() {
        let state = AppState::test_state(None);
        let repo = find_repo(&state, BOOTSTRAP_REPO_OWNER, BOOTSTRAP_REPO_NAME).unwrap();
        let identity = ShooIdentity {
            pairwise_sub: "pairwise-owner".to_string(),
            email: Some(BOOTSTRAP_OWNER_EMAIL.to_string()),
            email_verified: true,
        };
        let principal = principal_for_repo(&state, repo, Some(&identity));

        assert_eq!(principal.id, BOOTSTRAP_OWNER_USER_ID);
        assert_eq!(principal.kind, PrincipalKind::User);
    }

    #[test]
    fn unverified_bootstrap_email_uses_public_principal() {
        let state = AppState::test_state(None);
        let repo = find_repo(&state, BOOTSTRAP_REPO_OWNER, BOOTSTRAP_REPO_NAME).unwrap();
        let identity = ShooIdentity {
            pairwise_sub: "pairwise-owner".to_string(),
            email: Some(BOOTSTRAP_OWNER_EMAIL.to_string()),
            email_verified: false,
        };
        let principal = principal_for_repo(&state, repo, Some(&identity));

        assert_eq!(principal, Principal::public());
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

    #[test]
    fn manifest_signing_secret_is_absent_unless_explicitly_configured() {
        let state = AppState::test_state(None);

        assert!(state.manifest_signing_secret.is_none());
    }
}
