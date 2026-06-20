use crate::domain::policy::Principal;
use crate::domain::store::{AccountAccess, AppCatalog, StoredRepository, UserAccount};
use crate::{
    config::{LOCAL_APP_ORIGIN, SCOPE_APP_ORIGIN_ENV, SHOO_ISSUER, SHOO_JWKS_URL, non_empty_env},
    error::ApiError,
    http::responses::SessionIdentity,
    persistence::{lock_catalog, persist_catalog},
    state::AppState,
};
use axum::http::{HeaderMap, header::AUTHORIZATION};
use jsonwebtoken::{
    Algorithm, DecodingKey, Validation, decode, decode_header,
    jwk::{Jwk, JwkSet},
};
use serde::{Deserialize, Serialize};
use sha1::{Digest, Sha1};
use std::{
    sync::{Arc, Mutex},
    time::Duration,
};

#[derive(Clone)]
pub(crate) struct ShooVerifier {
    pub(crate) client: reqwest::Client,
    pub(crate) issuer: String,
    pub(crate) audience: Option<String>,
    pub(crate) jwks_url: String,
    pub(crate) jwks_cache: Arc<Mutex<Option<JwkSet>>>,
}

impl ShooVerifier {
    pub(crate) fn from_env() -> Self {
        Self::new(SHOO_ISSUER, shoo_audience_from_env(), SHOO_JWKS_URL)
    }

    pub(crate) fn new(
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

    pub(crate) async fn verify(&self, token: &str) -> Result<ShooIdentity, ApiError> {
        validated_shoo_header(token)?;
        let audience = self.audience.as_deref().ok_or_else(|| {
            ApiError::service_unavailable(format!(
                "Shoo auth requires {SCOPE_APP_ORIGIN_ENV} to match the web app origin"
            ))
        })?;
        let jwks = self.jwks().await?;

        verify_shoo_token(token, &jwks, &self.issuer, audience)
    }

    pub(crate) async fn jwks(&self) -> Result<JwkSet, ApiError> {
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
pub(crate) struct ShooIdentity {
    pub(crate) pairwise_sub: String,
    pub(crate) email: Option<String>,
    pub(crate) email_verified: bool,
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
pub(crate) struct ShooClaims {
    pub(crate) pairwise_sub: String,
    pub(crate) email: Option<String>,
    pub(crate) email_verified: Option<bool>,
}

pub(crate) fn verify_shoo_token(
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

pub(crate) async fn http_identity(
    state: &AppState,
    headers: &HeaderMap,
) -> Result<Option<ShooIdentity>, ApiError> {
    let Some(token) = bearer_token(headers)? else {
        return Ok(None);
    };

    state.shoo.verify(token).await.map(Some)
}

pub(crate) async fn require_identity(
    state: &AppState,
    headers: &HeaderMap,
) -> Result<ShooIdentity, ApiError> {
    http_identity(state, headers)
        .await?
        .ok_or_else(|| ApiError::unauthorized("sign in required"))
}

pub(crate) fn bearer_token(headers: &HeaderMap) -> Result<Option<&str>, ApiError> {
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

pub(crate) fn principal_for_repo(
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

pub(crate) fn ensure_user_for_identity(
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

pub(crate) fn principal_for_user_id(repo: &StoredRepository, user_id: &str) -> Principal {
    if repo
        .memberships
        .iter()
        .any(|membership| membership.user_id == user_id)
    {
        Principal {
            id: user_id.to_string(),
            kind: crate::domain::policy::PrincipalKind::User,
        }
    } else {
        Principal::public()
    }
}

pub(crate) fn identity_user_id(identity: &ShooIdentity) -> String {
    let digest = Sha1::digest(identity.pairwise_sub.as_bytes());
    let hex = format!("{digest:x}");
    format!("user_{}", &hex[..16])
}

pub(crate) fn preferred_user_handle(identity: &ShooIdentity) -> String {
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

pub(crate) fn unique_user_handle(catalog: &AppCatalog, preferred: &str, user_id: &str) -> String {
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

pub(crate) fn handle_is_available(catalog: &AppCatalog, handle: &str, user_id: &str) -> bool {
    catalog
        .users
        .values()
        .all(|user| user.id == user_id || user.handle != handle)
}

pub(crate) fn normalize_handle(value: &str) -> Option<String> {
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

pub(crate) fn normalize_email(email: &str) -> String {
    email.trim().to_ascii_lowercase()
}
