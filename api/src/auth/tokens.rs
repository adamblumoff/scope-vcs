use crate::{
    config::{FIRST_PUSH_TOKEN_PREFIX, GIT_PUSH_TOKEN_PREFIX},
    error::ApiError,
    persistence::unix_now,
};
use scope_domain::store::{FirstPushToken, GitPushToken};
use sha2::{Digest, Sha256};

const TOKEN_BYTES: usize = 32;
const FIRST_PUSH_TOKEN_TTL_SECS: u64 = 5 * 60;
const REPOSITORY_INVITE_TOKEN_PREFIX: &str = "scope_invite_";
const ATTEMPT_TOKEN_PREFIX: &str = "scope_attempt_";

pub(crate) fn generate_first_push_token(
    owner_user_id: &str,
) -> Result<(String, FirstPushToken), ApiError> {
    let now = unix_now()?;
    let secret = random_token(
        FIRST_PUSH_TOKEN_PREFIX,
        "failed to generate first-push token",
    )?;
    let token = FirstPushToken {
        token_hash: token_hash(&secret),
        secret: Some(secret.clone()),
        owner_user_id: owner_user_id.to_string(),
        created_at_unix: now,
        expires_at_unix: now + FIRST_PUSH_TOKEN_TTL_SECS,
        used_at_unix: None,
    };

    Ok((secret, token))
}

pub(crate) fn generate_git_push_token(
    owner_user_id: &str,
) -> Result<(String, GitPushToken), ApiError> {
    let now = unix_now()?;
    let secret = random_token(GIT_PUSH_TOKEN_PREFIX, "failed to generate Git push token")?;
    let token = GitPushToken {
        token_hash: token_hash(&secret),
        owner_user_id: owner_user_id.to_string(),
        created_at_unix: now,
    };

    Ok((secret, token))
}

pub(crate) fn generate_repository_invite_token() -> Result<(String, String), ApiError> {
    let secret = random_token(
        REPOSITORY_INVITE_TOKEN_PREFIX,
        "failed to generate repository invite token",
    )?;
    let hash = token_hash(&secret);
    Ok((secret, hash))
}

pub(crate) fn generate_attempt_token() -> Result<(String, String), ApiError> {
    generate_machine_token(ATTEMPT_TOKEN_PREFIX, "failed to generate attempt token")
}

fn generate_machine_token(
    prefix: &str,
    failure_message: &str,
) -> Result<(String, String), ApiError> {
    let secret = random_token(prefix, failure_message)?;
    let hash = machine_token_hash(&secret);
    Ok((secret, hash))
}

pub(crate) fn first_push_token_hash(secret: &str) -> String {
    token_hash(secret)
}

pub(crate) fn git_push_token_hash(secret: &str) -> String {
    token_hash(secret)
}

pub(crate) fn repository_invite_token_hash(secret: &str) -> String {
    token_hash(secret)
}

pub(crate) fn machine_token_hash(secret: &str) -> String {
    format!("{:x}", Sha256::digest(secret.as_bytes()))
}

pub(super) fn random_token(prefix: &str, failure_message: &str) -> Result<String, ApiError> {
    let mut bytes = [0_u8; TOKEN_BYTES];
    getrandom::fill(&mut bytes)
        .map_err(|error| ApiError::internal_message(format!("{failure_message}: {error}")))?;
    Ok(format!("{prefix}{}", hex::encode(bytes)))
}

pub(super) fn token_hash(secret: &str) -> String {
    let digest = Sha256::digest(secret.as_bytes());
    format!("sha256:{digest:x}")
}
