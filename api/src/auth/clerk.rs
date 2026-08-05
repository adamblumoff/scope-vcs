use crate::{
    config::{
        CLERK_AUDIENCE_ENV, CLERK_AUTHORIZED_PARTIES_ENV, CLERK_ISSUER_ENV, CLERK_JWKS_URL_ENV,
        DEFAULT_CLERK_AUDIENCE, LOCAL_APP_ORIGIN, SCOPE_APP_ORIGIN_ENV, non_empty_env,
    },
    error::ApiError,
};
use http::{HeaderMap, header::AUTHORIZATION};
use jsonwebtoken::{
    Algorithm, DecodingKey, Validation, decode, decode_header,
    jwk::{Jwk, JwkSet},
};
use serde::{Deserialize, Serialize};
use std::{
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};
use tokio::sync::Mutex as AsyncMutex;

const JWKS_FRESH_FOR: Duration = Duration::from_secs(5 * 60);
const JWKS_STALE_IF_ERROR_FOR: Duration = Duration::from_secs(5 * 60);
const JWKS_REFRESH_FAILURE_BACKOFF: Duration = Duration::from_secs(5);
const JWKS_UNKNOWN_KEY_REFRESH_COOLDOWN: Duration = Duration::from_secs(5);

#[derive(Clone)]
pub struct ClerkVerifier {
    pub client: reqwest::Client,
    pub issuer: Option<String>,
    pub jwks_url: Option<String>,
    pub token_policy: ClerkTokenPolicy,
    jwks_cache: Arc<JwksCache>,
}

struct JwksCache {
    state: Mutex<JwksCacheState>,
    refresh: AsyncMutex<()>,
    fresh_for: Duration,
    stale_if_error_for: Duration,
    unknown_key_refresh_cooldown: Duration,
}

#[derive(Default)]
struct JwksCacheState {
    current: Option<CachedJwks>,
    generation: u64,
    last_failure: Option<RefreshFailure>,
}

impl JwksCacheState {
    fn access(&self, source: JwksSource) -> JwksAccess {
        JwksAccess {
            keys: self
                .current
                .as_ref()
                .expect("cached JWKS access requires keys")
                .keys
                .clone(),
            generation: self.generation,
            source,
        }
    }
}

struct CachedJwks {
    keys: Arc<JwkSet>,
    fetched_at: Instant,
}

struct RefreshFailure {
    at: Instant,
    diagnostic: String,
}

struct JwksAccess {
    keys: Arc<JwkSet>,
    generation: u64,
    source: JwksSource,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum JwksSource {
    CachedFresh,
    Current,
    LastKnownGood,
}

#[derive(Clone, Copy)]
enum RefreshReason {
    Expired,
    UnknownKey,
}

impl ClerkVerifier {
    pub fn from_env() -> Self {
        let issuer =
            non_empty_env(CLERK_ISSUER_ENV).map(|value| value.trim_end_matches('/').to_string());
        let jwks_url = non_empty_env(CLERK_JWKS_URL_ENV).or_else(|| {
            issuer
                .as_ref()
                .map(|issuer| format!("{issuer}/.well-known/jwks.json"))
        });
        Self::new_with_policy(issuer, jwks_url, ClerkTokenPolicy::from_env())
    }

    pub fn new_with_policy(
        issuer: Option<String>,
        jwks_url: Option<String>,
        token_policy: ClerkTokenPolicy,
    ) -> Self {
        Self::new_with_cache_timing(
            issuer,
            jwks_url,
            token_policy,
            JWKS_FRESH_FOR,
            JWKS_STALE_IF_ERROR_FOR,
            JWKS_UNKNOWN_KEY_REFRESH_COOLDOWN,
        )
    }

    pub(crate) fn new_with_cache_timing(
        issuer: Option<String>,
        jwks_url: Option<String>,
        token_policy: ClerkTokenPolicy,
        fresh_for: Duration,
        stale_if_error_for: Duration,
        unknown_key_refresh_cooldown: Duration,
    ) -> Self {
        Self {
            client: reqwest::Client::builder()
                .timeout(Duration::from_secs(5))
                .build()
                .expect("Clerk verifier HTTP client config must be valid"),
            issuer,
            jwks_url,
            token_policy,
            jwks_cache: Arc::new(JwksCache {
                state: Mutex::new(JwksCacheState::default()),
                refresh: AsyncMutex::new(()),
                fresh_for,
                stale_if_error_for,
                unknown_key_refresh_cooldown,
            }),
        }
    }

    pub async fn verify(&self, token: &str) -> Result<ClerkIdentity, ApiError> {
        let issuer = self.issuer.as_deref().ok_or_else(|| {
            ApiError::infrastructure_unavailable(format!(
                "Clerk auth requires {CLERK_ISSUER_ENV} to be configured"
            ))
        })?;
        let header = validated_clerk_header(token)?;
        let kid = header
            .kid
            .as_deref()
            .expect("validated Clerk header must have a kid");
        let mut jwks = self.jwks().await?;

        if signing_key(kid, &jwks.keys).is_none() {
            jwks = match jwks.source {
                JwksSource::CachedFresh => {
                    self.refresh_jwks(jwks.generation, RefreshReason::UnknownKey)
                        .await?
                }
                JwksSource::Current => {
                    return Err(ApiError::unauthorized("Clerk signing key not found"));
                }
                JwksSource::LastKnownGood => {
                    return Err(ApiError::infrastructure_unavailable(
                        "Clerk JWKS refresh failed while resolving an unknown signing key",
                    ));
                }
            };
        }

        let jwk = signing_key(kid, &jwks.keys)
            .ok_or_else(|| ApiError::unauthorized("Clerk signing key not found"))?;

        verify_clerk_token_with_header(token, &header, jwk, issuer, &self.token_policy)
    }

    async fn jwks(&self) -> Result<JwksAccess, ApiError> {
        let now = Instant::now();
        let generation = {
            let state = self.cache_state();
            if let Some(current) = state.current.as_ref()
                && now.duration_since(current.fetched_at) < self.jwks_cache.fresh_for
            {
                return Ok(state.access(JwksSource::CachedFresh));
            }
            state.generation
        };

        self.refresh_jwks(generation, RefreshReason::Expired).await
    }

    async fn refresh_jwks(
        &self,
        observed_generation: u64,
        reason: RefreshReason,
    ) -> Result<JwksAccess, ApiError> {
        let _refresh = self.jwks_cache.refresh.lock().await;
        let now = Instant::now();

        {
            let state = self.cache_state();
            if state.generation > observed_generation {
                return Ok(state.access(JwksSource::Current));
            }
            if matches!(reason, RefreshReason::Expired)
                && let Some(current) = state.current.as_ref()
                && now.duration_since(current.fetched_at) < self.jwks_cache.fresh_for
            {
                return Ok(state.access(JwksSource::CachedFresh));
            }
            if matches!(reason, RefreshReason::UnknownKey)
                && let Some(current) = state.current.as_ref()
                && now.duration_since(current.fetched_at)
                    < self.jwks_cache.unknown_key_refresh_cooldown
            {
                return Ok(state.access(JwksSource::Current));
            }
            if let Some(failure) = state.last_failure.as_ref()
                && now.duration_since(failure.at) < JWKS_REFRESH_FAILURE_BACKOFF
            {
                return self.last_known_good_or_error(&state, now, reason, &failure.diagnostic);
            }
        }

        let fetched = self.fetch_jwks().await;
        let now = Instant::now();
        let mut state = self.cache_state();
        match fetched {
            Ok(keys) => {
                state.generation = state.generation.saturating_add(1);
                state.current = Some(CachedJwks {
                    keys: Arc::new(keys),
                    fetched_at: now,
                });
                state.last_failure = None;
                Ok(state.access(JwksSource::Current))
            }
            Err(error) => {
                let diagnostic = error.into_operator_diagnostic();
                state.last_failure = Some(RefreshFailure {
                    at: now,
                    diagnostic: diagnostic.clone(),
                });
                self.last_known_good_or_error(&state, now, reason, &diagnostic)
            }
        }
    }

    async fn fetch_jwks(&self) -> Result<JwkSet, ApiError> {
        let jwks_url = self.jwks_url.as_deref().ok_or_else(|| {
            ApiError::infrastructure_unavailable(format!(
                "Clerk auth requires {CLERK_JWKS_URL_ENV} or {CLERK_ISSUER_ENV}"
            ))
        })?;
        self.client
            .get(jwks_url)
            .send()
            .await
            .map_err(|error| {
                ApiError::infrastructure_unavailable(format!("failed to fetch Clerk JWKS: {error}"))
            })?
            .error_for_status()
            .map_err(|error| {
                ApiError::infrastructure_unavailable(format!("failed to fetch Clerk JWKS: {error}"))
            })?
            .json::<JwkSet>()
            .await
            .map_err(|error| {
                ApiError::infrastructure_unavailable(format!(
                    "failed to decode Clerk JWKS: {error}"
                ))
            })
    }

    fn last_known_good_or_error(
        &self,
        state: &JwksCacheState,
        now: Instant,
        reason: RefreshReason,
        diagnostic: &str,
    ) -> Result<JwksAccess, ApiError> {
        if matches!(reason, RefreshReason::Expired)
            && let Some(current) = state.current.as_ref()
            && now.duration_since(current.fetched_at)
                < self
                    .jwks_cache
                    .fresh_for
                    .saturating_add(self.jwks_cache.stale_if_error_for)
        {
            return Ok(state.access(JwksSource::LastKnownGood));
        }

        Err(ApiError::infrastructure_unavailable(diagnostic))
    }

    fn cache_state(&self) -> std::sync::MutexGuard<'_, JwksCacheState> {
        self.jwks_cache
            .state
            .lock()
            .expect("Clerk JWKS cache lock must not be poisoned")
    }

    #[cfg(test)]
    pub(crate) fn cache_jwks_for_tests(&self, keys: JwkSet) {
        let mut state = self.cache_state();
        state.generation = state.generation.saturating_add(1);
        state.current = Some(CachedJwks {
            keys: Arc::new(keys),
            fetched_at: Instant::now(),
        });
        state.last_failure = None;
    }
}

pub type ClerkIdentity = scope_domain::account::ExternalIdentity;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ClerkTokenPolicy {
    pub authorized_parties: Vec<String>,
    pub audiences: Vec<String>,
}

impl Default for ClerkTokenPolicy {
    fn default() -> Self {
        Self {
            authorized_parties: Vec::new(),
            audiences: vec![DEFAULT_CLERK_AUDIENCE.to_string()],
        }
    }
}

impl ClerkTokenPolicy {
    pub fn from_env() -> Self {
        Self {
            authorized_parties: configured_authorized_parties(),
            audiences: configured_audiences(),
        }
    }

    fn validate(&self, claims: &ClerkClaims) -> Result<(), ApiError> {
        if self.authorized_parties.is_empty() && self.audiences.is_empty() {
            return Err(ApiError::infrastructure_unavailable(format!(
                "{CLERK_AUTHORIZED_PARTIES_ENV}, {SCOPE_APP_ORIGIN_ENV}, or {CLERK_AUDIENCE_ENV} is required to validate Clerk tokens"
            )));
        }

        let audience_allowed = !self.audiences.is_empty()
            && claims
                .aud
                .as_ref()
                .is_some_and(|audience| audience.matches_any(&self.audiences));
        if !self.audiences.is_empty() && !audience_allowed {
            return Err(ApiError::unauthorized(
                "Clerk token audience is not allowed",
            ));
        }

        let Some(azp) = claims.azp.as_deref().map(normalize_claim_value) else {
            if audience_allowed {
                return Ok(());
            }
            return Err(ApiError::unauthorized(
                "Clerk token is missing authorized party",
            ));
        };

        if self.authorized_parties.is_empty() {
            return Ok(());
        }
        if !self
            .authorized_parties
            .iter()
            .any(|allowed| allowed == &azp)
        {
            return Err(ApiError::unauthorized(
                "Clerk token authorized party is not allowed",
            ));
        }

        Ok(())
    }
}

#[derive(Debug, Deserialize, Serialize)]
pub struct ClerkClaims {
    pub sub: String,
    pub email: Option<String>,
    pub email_verified: Option<bool>,
    pub aud: Option<AudienceClaim>,
    pub azp: Option<String>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(untagged)]
pub enum AudienceClaim {
    One(String),
    Many(Vec<String>),
}

impl AudienceClaim {
    fn matches_any(&self, allowed: &[String]) -> bool {
        match self {
            AudienceClaim::One(value) => allowed.iter().any(|allowed| allowed == value),
            AudienceClaim::Many(values) => values
                .iter()
                .any(|value| allowed.iter().any(|allowed| allowed == value)),
        }
    }
}

#[cfg(test)]
pub fn verify_clerk_token(
    token: &str,
    jwks: &JwkSet,
    issuer: &str,
    token_policy: &ClerkTokenPolicy,
) -> Result<ClerkIdentity, ApiError> {
    let header = validated_clerk_header(token)?;
    let kid = header
        .kid
        .as_deref()
        .expect("validated Clerk header must have a kid");
    let jwk = signing_key(kid, jwks)
        .ok_or_else(|| ApiError::unauthorized("Clerk signing key not found"))?;
    verify_clerk_token_with_header(token, &header, jwk, issuer, token_policy)
}

fn verify_clerk_token_with_header(
    token: &str,
    header: &jsonwebtoken::Header,
    jwk: &Jwk,
    issuer: &str,
    token_policy: &ClerkTokenPolicy,
) -> Result<ClerkIdentity, ApiError> {
    let key = DecodingKey::from_jwk(jwk).map_err(ApiError::internal)?;
    let mut validation = Validation::new(header.alg);
    validation.validate_aud = false;
    validation.set_required_spec_claims(&["exp", "iss", "sub"]);
    validation.set_issuer(&[issuer]);

    let claims = decode::<ClerkClaims>(token, &key, &validation)
        .map_err(|_| ApiError::unauthorized("invalid Clerk token"))?
        .claims;

    token_policy.validate(&claims)?;

    if claims.sub.trim().is_empty() {
        return Err(ApiError::unauthorized("Clerk token is missing sub"));
    }

    Ok(ClerkIdentity {
        provider: "clerk".to_string(),
        subject: claims.sub,
        email: claims.email,
        email_verified: claims.email_verified.unwrap_or(false),
    })
}

fn validated_clerk_header(token: &str) -> Result<jsonwebtoken::Header, ApiError> {
    let header =
        decode_header(token).map_err(|_| ApiError::unauthorized("invalid bearer token"))?;
    if !matches!(header.alg, Algorithm::ES256 | Algorithm::RS256) {
        return Err(ApiError::unauthorized("unsupported Clerk token algorithm"));
    }
    if header.kid.is_none() {
        return Err(ApiError::unauthorized("Clerk token is missing kid"));
    }

    Ok(header)
}

fn signing_key<'a>(kid: &str, jwks: &'a JwkSet) -> Option<&'a Jwk> {
    jwks.keys
        .iter()
        .find(|jwk| jwk.common.key_id.as_deref() == Some(kid))
}

fn configured_authorized_parties() -> Vec<String> {
    let mut values = configured_list(CLERK_AUTHORIZED_PARTIES_ENV);
    if values.is_empty() {
        values
            .extend(non_empty_env(SCOPE_APP_ORIGIN_ENV).map(|value| normalize_claim_value(&value)));
        if cfg!(debug_assertions) {
            values.push(LOCAL_APP_ORIGIN.to_string());
        }
    }
    values.sort();
    values.dedup();
    values
}

fn configured_audiences() -> Vec<String> {
    let audiences = configured_list(CLERK_AUDIENCE_ENV);
    if audiences.is_empty() {
        vec![DEFAULT_CLERK_AUDIENCE.to_string()]
    } else {
        audiences
    }
}

fn configured_list(name: &str) -> Vec<String> {
    non_empty_env(name)
        .into_iter()
        .flat_map(|value| {
            value
                .split(',')
                .map(normalize_claim_value)
                .filter(|value| !value.is_empty())
                .collect::<Vec<_>>()
        })
        .collect()
}

fn normalize_claim_value(value: &str) -> String {
    value.trim().trim_end_matches('/').to_string()
}

pub fn bearer_token(headers: &HeaderMap) -> Result<Option<&str>, ApiError> {
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
