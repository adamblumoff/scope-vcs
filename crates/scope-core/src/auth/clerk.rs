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
    time::Duration,
};

#[derive(Clone)]
pub struct ClerkVerifier {
    pub client: reqwest::Client,
    pub issuer: Option<String>,
    pub jwks_url: Option<String>,
    pub token_policy: ClerkTokenPolicy,
    pub jwks_cache: Arc<Mutex<Option<JwkSet>>>,
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
        Self {
            client: reqwest::Client::builder()
                .timeout(Duration::from_secs(5))
                .build()
                .expect("Clerk verifier HTTP client config must be valid"),
            issuer,
            jwks_url,
            token_policy,
            jwks_cache: Arc::new(Mutex::new(None)),
        }
    }

    pub async fn verify(&self, token: &str) -> Result<ClerkIdentity, ApiError> {
        let issuer = self.issuer.as_deref().ok_or_else(|| {
            ApiError::service_unavailable(format!(
                "Clerk auth requires {CLERK_ISSUER_ENV} to be configured"
            ))
        })?;
        let jwks = self.jwks().await?;

        verify_clerk_token(token, &jwks, issuer, &self.token_policy)
    }

    pub async fn jwks(&self) -> Result<JwkSet, ApiError> {
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
pub struct ClerkIdentity {
    pub user_id: String,
    pub email: Option<String>,
    pub email_verified: bool,
}

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
            return Err(ApiError::service_unavailable(format!(
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

pub fn verify_clerk_token(
    token: &str,
    jwks: &JwkSet,
    issuer: &str,
    token_policy: &ClerkTokenPolicy,
) -> Result<ClerkIdentity, ApiError> {
    let header = validated_clerk_header(token)?;
    let jwk = signing_key(&header.kid, jwks)?;
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
