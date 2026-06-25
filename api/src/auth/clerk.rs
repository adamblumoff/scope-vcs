use crate::domain::policy::Principal;
use crate::domain::store::{AccountAccess, AppCatalog, StoredRepository, UserAccount};
use crate::{
    config::{CLERK_ISSUER_ENV, CLERK_JWKS_URL_ENV, CLI_ACCESS_TOKEN_PREFIX, non_empty_env},
    error::ApiError,
    http::responses::SessionIdentity,
    state::AppState,
};
use axum::http::{HeaderMap, header::AUTHORIZATION};
use jsonwebtoken::{
    Algorithm, DecodingKey, Validation, decode, decode_header,
    jwk::{Jwk, JwkSet},
};
use serde::{Deserialize, Serialize};
use std::{
    sync::{Arc, Mutex},
    time::Duration,
};

#[derive(Clone)]
pub(crate) struct ClerkVerifier {
    pub(crate) client: reqwest::Client,
    pub(crate) issuer: Option<String>,
    pub(crate) jwks_url: Option<String>,
    pub(crate) jwks_cache: Arc<Mutex<Option<JwkSet>>>,
}

impl ClerkVerifier {
    pub(crate) fn from_env() -> Self {
        let issuer =
            non_empty_env(CLERK_ISSUER_ENV).map(|value| value.trim_end_matches('/').to_string());
        let jwks_url = non_empty_env(CLERK_JWKS_URL_ENV).or_else(|| {
            issuer
                .as_ref()
                .map(|issuer| format!("{issuer}/.well-known/jwks.json"))
        });
        Self::new(issuer, jwks_url)
    }

    pub(crate) fn new(issuer: Option<String>, jwks_url: Option<String>) -> Self {
        Self {
            client: reqwest::Client::builder()
                .timeout(Duration::from_secs(5))
                .build()
                .expect("Clerk verifier HTTP client config must be valid"),
            issuer,
            jwks_url,
            jwks_cache: Arc::new(Mutex::new(None)),
        }
    }

    pub(crate) async fn verify(&self, token: &str) -> Result<ClerkIdentity, ApiError> {
        let issuer = self.issuer.as_deref().ok_or_else(|| {
            ApiError::service_unavailable(format!(
                "Clerk auth requires {CLERK_ISSUER_ENV} to be configured"
            ))
        })?;
        let jwks = self.jwks().await?;

        verify_clerk_token(token, &jwks, issuer)
    }

    pub(crate) async fn jwks(&self) -> Result<JwkSet, ApiError> {
        if let Some(jwks) = self
            .jwks_cache
            .lock()
            .expect("Clerk JWKS cache lock must not be poisoned")
            .clone()
        {
            return Ok(jwks);
        }

        let jwks_url = self.jwks_url.as_deref().ok_or_else(|| {
            ApiError::service_unavailable(format!(
                "Clerk auth requires {CLERK_JWKS_URL_ENV} or {CLERK_ISSUER_ENV}"
            ))
        })?;
        let jwks = self
            .client
            .get(jwks_url)
            .send()
            .await
            .map_err(|error| {
                ApiError::service_unavailable(format!("failed to fetch Clerk JWKS: {error}"))
            })?
            .error_for_status()
            .map_err(|error| {
                ApiError::service_unavailable(format!("failed to fetch Clerk JWKS: {error}"))
            })?
            .json::<JwkSet>()
            .await
            .map_err(ApiError::internal)?;

        *self
            .jwks_cache
            .lock()
            .expect("Clerk JWKS cache lock must not be poisoned") = Some(jwks.clone());
        Ok(jwks)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ClerkIdentity {
    pub(crate) user_id: String,
    pub(crate) email: Option<String>,
    pub(crate) email_verified: bool,
}

impl From<&ClerkIdentity> for SessionIdentity {
    fn from(identity: &ClerkIdentity) -> Self {
        Self {
            user_id: identity.user_id.clone(),
            email: identity.email.clone(),
            email_verified: identity.email_verified,
        }
    }
}

#[derive(Debug, Deserialize, Serialize)]
pub(crate) struct ClerkClaims {
    pub(crate) sub: String,
    pub(crate) email: Option<String>,
    pub(crate) email_verified: Option<bool>,
}

pub(crate) fn verify_clerk_token(
    token: &str,
    jwks: &JwkSet,
    issuer: &str,
) -> Result<ClerkIdentity, ApiError> {
    let header = validated_clerk_header(token)?;
    let jwk = signing_key(&header.kid, jwks)?;
    let key = DecodingKey::from_jwk(jwk).map_err(ApiError::internal)?;
    let mut validation = Validation::new(header.alg);
    validation.set_required_spec_claims(&["exp", "iss", "sub"]);
    validation.set_issuer(&[issuer]);

    let claims = decode::<ClerkClaims>(token, &key, &validation)
        .map_err(|_| ApiError::unauthorized("invalid Clerk token"))?
        .claims;

    if claims.sub.trim().is_empty() {
        return Err(ApiError::unauthorized("Clerk token is missing sub"));
    }

    Ok(ClerkIdentity {
        user_id: claims.sub,
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

fn signing_key<'a>(kid: &Option<String>, jwks: &'a JwkSet) -> Result<&'a Jwk, ApiError> {
    let Some(kid) = kid.as_deref() else {
        return Err(ApiError::unauthorized("Clerk token is missing kid"));
    };

    jwks.keys
        .iter()
        .find(|jwk| jwk.common.key_id.as_deref() == Some(kid))
        .ok_or_else(|| ApiError::unauthorized("Clerk signing key not found"))
}

pub(crate) async fn http_identity(
    state: &AppState,
    headers: &HeaderMap,
) -> Result<Option<ClerkIdentity>, ApiError> {
    let Some(token) = bearer_token(headers)? else {
        return Ok(None);
    };

    if token.starts_with(CLI_ACCESS_TOKEN_PREFIX) {
        return state.device_logins.verify_access_token(token).map(Some);
    }

    state.clerk.verify(token).await.map(Some)
}

pub(crate) async fn require_identity(
    state: &AppState,
    headers: &HeaderMap,
) -> Result<ClerkIdentity, ApiError> {
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
    identity: Option<&ClerkIdentity>,
) -> Result<Principal, ApiError> {
    let Some(identity) = identity else {
        return Ok(Principal::public());
    };

    let user = ensure_user_for_identity(state, identity)?;
    Ok(principal_for_user_id(repo, &user.id))
}

pub(crate) fn ensure_user_for_identity(
    state: &AppState,
    identity: &ClerkIdentity,
) -> Result<UserAccount, ApiError> {
    let user_id = identity_user_id(identity);
    let email = identity
        .email
        .as_deref()
        .map(normalize_email)
        .unwrap_or_default();
    let preferred_handle = preferred_user_handle(identity);
    let email_verified = identity.email_verified;

    state.metadata.update(move |catalog| {
        let user = match catalog.users.get_mut(&user_id) {
            Some(user) => {
                user.email = email;
                user.email_verified = email_verified;
                user.access = AccountAccess::Member;
                user.clone()
            }
            None => {
                let handle = unique_user_handle(catalog, &preferred_handle, &user_id);
                let user = UserAccount {
                    id: user_id.clone(),
                    handle,
                    email,
                    email_verified,
                    access: AccountAccess::Member,
                };
                catalog.users.insert(user_id.clone(), user.clone());
                user
            }
        };

        Ok(user)
    })
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

pub(crate) fn identity_user_id(identity: &ClerkIdentity) -> String {
    identity.user_id.clone()
}

pub(crate) fn preferred_user_handle(identity: &ClerkIdentity) -> String {
    let fallback = handle_suffix(&identity.user_id);
    let raw = identity
        .email
        .as_deref()
        .filter(|_| identity.email_verified)
        .and_then(|email| email.split('@').next())
        .filter(|local| !local.trim().is_empty())
        .unwrap_or(&fallback);

    normalize_handle(raw).unwrap_or(fallback)
}

fn handle_suffix(user_id: &str) -> String {
    let suffix = user_id
        .chars()
        .filter(|ch| ch.is_ascii_alphanumeric())
        .take(8)
        .collect::<String>();
    if suffix.is_empty() {
        "user".to_string()
    } else {
        format!("user-{suffix}")
    }
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
