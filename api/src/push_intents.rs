use crate::{
    config::non_empty_env,
    error::ApiError,
    persistence::{ensure_private_dir, unix_now},
    state::AppState,
};
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use hmac::{Hmac, Mac};
use scope_domain::{
    content_ref::ContentRef,
    repo_config::{RepoConfig, repo_config_fingerprint as domain_repo_config_fingerprint},
};
use serde::{Deserialize, Serialize};
use sha2::Sha256;
use std::{fs, path::Path, sync::Arc};

const PUSH_INTENT_TTL_SECS: u64 = 10 * 60;
const PUSH_INTENT_TOKEN_PREFIX: &str = "scope_pi_";
const PUSH_INTENT_KIND: &str = "scope.push-intent";
const PUSH_INTENT_VERSION: u8 = 1;
const PUSH_INTENT_SIGNING_KEY_ENV: &str = "SCOPE_PUSH_INTENT_SIGNING_KEY";
const PUSH_INTENT_SIGNING_KEY_FILE: &str = "push-intent-signing-key";
const PUSH_INTENT_KEY_DERIVATION_CONTEXT: &[u8] = b"scope.push-intent.signing-key.v1";
type HmacSha256 = Hmac<Sha256>;

#[derive(Clone, Debug, Deserialize, Serialize)]
struct PushIntentClaims {
    kind: String,
    version: u8,
    repo_id: String,
    user_id: String,
    head_oid: String,
    config: RepoConfig,
    base_config_hash: String,
    base_git_manifest_ref: Option<ContentRef>,
    expires_at_unix: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ValidatedPushIntent {
    pub(crate) repo_id: String,
    pub(crate) user_id: String,
    pub(crate) head_oid: String,
    pub(crate) config: RepoConfig,
    pub(crate) base_config_hash: String,
    pub(crate) base_git_manifest_ref: Option<ContentRef>,
    pub(crate) expires_at_unix: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct CreatedPushIntent {
    pub(crate) token: String,
    pub(crate) expires_at_unix: u64,
}

impl ValidatedPushIntent {
    pub(crate) fn ensure_repo_user(&self, repo_id: &str, user_id: &str) -> Result<(), ApiError> {
        if self.repo_id == repo_id && self.user_id == user_id {
            Ok(())
        } else {
            Err(ApiError::forbidden(
                "Scope push intent does not match received Git push",
            ))
        }
    }

    pub(crate) fn base_for_head(&self, head_oid: &str) -> Result<Option<ContentRef>, ApiError> {
        if self.head_oid == head_oid {
            Ok(self.base_git_manifest_ref.clone())
        } else {
            Err(ApiError::forbidden(
                "Scope push intent does not match received Git push",
            ))
        }
    }
}

impl AppState {
    pub(crate) fn create_push_intent(
        &self,
        repo_id: &str,
        user_id: &str,
        head_oid: &str,
        config: RepoConfig,
        base_config_hash: String,
        base_git_manifest_ref: Option<ContentRef>,
    ) -> Result<CreatedPushIntent, ApiError> {
        let expires_at_unix = unix_now()?.saturating_add(PUSH_INTENT_TTL_SECS);
        let intent = PushIntentClaims {
            kind: PUSH_INTENT_KIND.to_string(),
            version: PUSH_INTENT_VERSION,
            repo_id: repo_id.to_string(),
            user_id: user_id.to_string(),
            head_oid: head_oid.to_string(),
            config,
            base_config_hash,
            base_git_manifest_ref,
            expires_at_unix,
        };
        let token = encode_push_intent(&self.push_intent_signing_key, &intent)?;
        Ok(CreatedPushIntent {
            token,
            expires_at_unix,
        })
    }

    pub(crate) fn validate_push_intent_secret(
        &self,
        secret: &str,
    ) -> Result<ValidatedPushIntent, ApiError> {
        decode_push_intent(&self.push_intent_signing_key, secret, true)
            .map(validated_push_intent_from_claims)
    }
}

pub(crate) fn push_intent_signing_key(
    data_dir: &Path,
    shared_root_key: Option<&[u8]>,
) -> Result<Arc<[u8]>, ApiError> {
    if let Some(secret) = non_empty_env(PUSH_INTENT_SIGNING_KEY_ENV) {
        return Ok(Arc::from(secret.into_bytes()));
    }
    if let Some(shared_root_key) = shared_root_key {
        return derive_push_intent_signing_key(shared_root_key);
    }

    ensure_private_dir(data_dir)?;
    let key_path = data_dir.join(PUSH_INTENT_SIGNING_KEY_FILE);
    if key_path.exists() {
        let secret = fs::read_to_string(&key_path).map_err(ApiError::internal)?;
        let secret = secret.trim();
        if secret.is_empty() {
            return Err(ApiError::internal_message(
                "push intent signing key file is empty",
            ));
        }
        return Ok(Arc::from(secret.as_bytes()));
    }

    let mut bytes = [0u8; 32];
    getrandom::fill(&mut bytes).map_err(|error| {
        ApiError::internal_message(format!(
            "push intent signing key generation failed: {error}"
        ))
    })?;
    let secret = URL_SAFE_NO_PAD.encode(bytes);
    fs::write(&key_path, format!("{secret}\n")).map_err(ApiError::internal)?;
    Ok(Arc::from(secret.into_bytes()))
}

fn derive_push_intent_signing_key(shared_root_key: &[u8]) -> Result<Arc<[u8]>, ApiError> {
    let mut mac = HmacSha256::new_from_slice(shared_root_key).map_err(ApiError::internal)?;
    mac.update(PUSH_INTENT_KEY_DERIVATION_CONTEXT);
    Ok(Arc::from(mac.finalize().into_bytes().to_vec()))
}

pub(crate) fn repo_config_fingerprint(config: &RepoConfig) -> Result<String, ApiError> {
    domain_repo_config_fingerprint(config).map_err(ApiError::internal)
}

fn encode_push_intent(signing_key: &[u8], intent: &PushIntentClaims) -> Result<String, ApiError> {
    let payload = serde_json::to_vec(intent).map_err(ApiError::internal)?;
    let payload = URL_SAFE_NO_PAD.encode(payload);
    let signature = sign_push_intent(signing_key, payload.as_bytes())?;
    Ok(format!(
        "{PUSH_INTENT_TOKEN_PREFIX}{payload}.{}",
        URL_SAFE_NO_PAD.encode(signature)
    ))
}

fn validated_push_intent_from_claims(intent: PushIntentClaims) -> ValidatedPushIntent {
    ValidatedPushIntent {
        repo_id: intent.repo_id,
        user_id: intent.user_id,
        head_oid: intent.head_oid,
        config: intent.config,
        base_config_hash: intent.base_config_hash,
        base_git_manifest_ref: intent.base_git_manifest_ref,
        expires_at_unix: intent.expires_at_unix,
    }
}

fn decode_push_intent(
    signing_key: &[u8],
    token: &str,
    enforce_expiry: bool,
) -> Result<PushIntentClaims, ApiError> {
    let Some(token) = token.trim().strip_prefix(PUSH_INTENT_TOKEN_PREFIX) else {
        return Err(ApiError::forbidden("valid Scope push intent required"));
    };
    let Some((payload, signature)) = token.split_once('.') else {
        return Err(ApiError::forbidden("valid Scope push intent required"));
    };
    let signature = URL_SAFE_NO_PAD
        .decode(signature)
        .map_err(|_| ApiError::forbidden("valid Scope push intent required"))?;
    verify_push_intent_signature(signing_key, payload.as_bytes(), &signature)?;
    let payload = URL_SAFE_NO_PAD
        .decode(payload)
        .map_err(|_| ApiError::forbidden("valid Scope push intent required"))?;
    let intent: PushIntentClaims = serde_json::from_slice(&payload)
        .map_err(|_| ApiError::forbidden("valid Scope push intent required"))?;
    if intent.kind != PUSH_INTENT_KIND || intent.version != PUSH_INTENT_VERSION {
        return Err(ApiError::forbidden("valid Scope push intent required"));
    }
    if enforce_expiry && intent.expires_at_unix <= unix_now()? {
        return Err(ApiError::forbidden("valid Scope push intent required"));
    }
    Ok(intent)
}

fn sign_push_intent(signing_key: &[u8], payload: &[u8]) -> Result<Vec<u8>, ApiError> {
    let mut mac = HmacSha256::new_from_slice(signing_key).map_err(ApiError::internal)?;
    mac.update(payload);
    Ok(mac.finalize().into_bytes().to_vec())
}

fn verify_push_intent_signature(
    signing_key: &[u8],
    payload: &[u8],
    signature: &[u8],
) -> Result<(), ApiError> {
    let mut mac = HmacSha256::new_from_slice(signing_key).map_err(ApiError::internal)?;
    mac.update(payload);
    mac.verify_slice(signature)
        .map_err(|_| ApiError::forbidden("valid Scope push intent required"))
}

#[cfg(test)]
mod tests {
    use super::derive_push_intent_signing_key;

    #[test]
    fn shared_root_key_derives_one_domain_separated_signing_key() {
        let root = [7_u8; 32];
        let first = derive_push_intent_signing_key(&root).unwrap();
        let second = derive_push_intent_signing_key(&root).unwrap();
        let other = derive_push_intent_signing_key(&[8_u8; 32]).unwrap();

        assert_eq!(first.as_ref(), second.as_ref());
        assert_ne!(first.as_ref(), root.as_slice());
        assert_ne!(first.as_ref(), other.as_ref());
    }
}
