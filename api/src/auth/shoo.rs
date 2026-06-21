use crate::domain::policy::Principal;
use crate::domain::store::{
    AccountAccess, AppCatalog, RepoMembership, StoredRepository, UserAccount,
};
use crate::{
    config::{LOCAL_APP_ORIGIN, SCOPE_APP_ORIGIN_ENV, SHOO_ISSUER, SHOO_JWKS_URL, non_empty_env},
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

    let preferred_handle = preferred_user_handle(identity);
    let email_verified = identity.email_verified;
    state.metadata.update(move |catalog| {
        let account_id = verified_email_account_id(catalog, &email, email_verified)
            .unwrap_or_else(|| user_id.clone());

        let user = match catalog.users.get_mut(&account_id) {
            Some(user) => {
                user.email = email;
                user.email_verified = email_verified;
                user.access = AccountAccess::Member;
                user.clone()
            }
            None => {
                let handle = unique_user_handle(catalog, &preferred_handle, &account_id);
                let user = UserAccount {
                    id: account_id.clone(),
                    handle,
                    email,
                    email_verified,
                    access: AccountAccess::Member,
                };
                catalog.users.insert(account_id.clone(), user.clone());
                user
            }
        };

        if email_verified && !user.email.is_empty() {
            if user_id != user.id && catalog.users.contains_key(&user_id) {
                reassign_user_references(catalog, &user_id, &user.id);
                catalog.users.remove(&user_id);
            }
            merge_verified_email_duplicates(catalog, &user.id, &user.email);
        }

        Ok(user)
    })
}

fn verified_email_account_id(
    catalog: &AppCatalog,
    email: &str,
    email_verified: bool,
) -> Option<String> {
    if !email_verified || email.is_empty() {
        return None;
    }

    let mut candidates = catalog
        .users
        .values()
        .filter(|user| user.email_verified && user.email == email)
        .map(|user| user.id.clone())
        .collect::<Vec<_>>();
    if candidates.is_empty() {
        return None;
    }

    candidates.sort_by(|left, right| {
        user_canonical_rank(catalog, right)
            .cmp(&user_canonical_rank(catalog, left))
            .then_with(|| left.cmp(right))
    });

    candidates.into_iter().next()
}

fn merge_verified_email_duplicates(catalog: &mut AppCatalog, canonical_id: &str, email: &str) {
    let duplicate_ids = catalog
        .users
        .values()
        .filter(|user| user.id != canonical_id && user.email_verified && user.email == email)
        .map(|user| user.id.clone())
        .collect::<Vec<_>>();

    for duplicate_id in duplicate_ids {
        reassign_user_references(catalog, &duplicate_id, canonical_id);
        catalog.users.remove(&duplicate_id);
    }
}

fn reassign_user_references(catalog: &mut AppCatalog, old_id: &str, new_id: &str) {
    for repo in catalog.repositories.values_mut() {
        if repo.record.owner_user_id == old_id {
            repo.record.owner_user_id = new_id.to_string();
        }
        if let Some(token) = repo.first_push_token.as_mut()
            && token.owner_user_id == old_id
        {
            token.owner_user_id = new_id.to_string();
        }
        if let Some(token) = repo.git_push_token.as_mut()
            && token.owner_user_id == old_id
        {
            token.owner_user_id = new_id.to_string();
        }
        if let Some(update) = repo.staged_update.as_mut()
            && update.author_id == old_id
        {
            update.author_id = new_id.to_string();
        }

        repo.policy.reassign_principal(old_id, new_id);
        for commit in &mut repo.graph.commits {
            if commit.author_id == old_id {
                commit.author_id = new_id.to_string();
            }
        }
        for invitation in &mut repo.invitations {
            if invitation.invited_by_user_id == old_id {
                invitation.invited_by_user_id = new_id.to_string();
            }
        }
        for membership in &mut repo.memberships {
            if membership.user_id == old_id {
                membership.user_id = new_id.to_string();
            }
        }
        dedupe_memberships(&mut repo.memberships);
    }
}

fn dedupe_memberships(memberships: &mut Vec<RepoMembership>) {
    let mut by_user = std::collections::BTreeMap::<String, RepoMembership>::new();
    for membership in memberships.drain(..) {
        by_user
            .entry(membership.user_id.clone())
            .and_modify(|existing| {
                if existing.role < membership.role {
                    existing.role = membership.role;
                }
            })
            .or_insert(membership);
    }
    memberships.extend(by_user.into_values());
}

fn user_reference_count(catalog: &AppCatalog, user_id: &str) -> usize {
    catalog
        .repositories
        .values()
        .map(|repo| {
            usize::from(repo.record.owner_user_id == user_id)
                + repo
                    .memberships
                    .iter()
                    .filter(|membership| membership.user_id == user_id)
                    .count()
        })
        .sum()
}

fn user_canonical_rank(catalog: &AppCatalog, user_id: &str) -> (usize, usize) {
    (
        user_reference_count(catalog, user_id),
        catalog
            .users
            .get(user_id)
            .map(|user| usize::from(!has_numeric_handle_suffix(&user.handle)))
            .unwrap_or_default(),
    )
}

fn has_numeric_handle_suffix(handle: &str) -> bool {
    handle
        .rsplit_once('-')
        .is_some_and(|(_, suffix)| suffix.parse::<u64>().is_ok())
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
