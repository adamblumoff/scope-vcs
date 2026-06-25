use crate::domain::store::{FirstPushToken, GitCloneToken, GitPushToken};
use crate::{
    config::{
        FIRST_PUSH_TOKEN_BYTES, FIRST_PUSH_TOKEN_PREFIX, FIRST_PUSH_TOKEN_TTL_SECS,
        GIT_PUSH_TOKEN_PREFIX,
    },
    error::ApiError,
    persistence::unix_now,
};
use sha2::{Digest, Sha256};

pub(crate) fn generate_first_push_token(
    owner_user_id: &str,
) -> Result<(String, FirstPushToken), ApiError> {
    let now = unix_now()?;
    let mut bytes = [0_u8; FIRST_PUSH_TOKEN_BYTES];
    getrandom::fill(&mut bytes).map_err(|error| {
        ApiError::internal_message(format!("failed to generate first-push token: {error}"))
    })?;
    let secret = format!("{FIRST_PUSH_TOKEN_PREFIX}{}", hex::encode(bytes));
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
    let mut bytes = [0_u8; FIRST_PUSH_TOKEN_BYTES];
    getrandom::fill(&mut bytes).map_err(|error| {
        ApiError::internal_message(format!("failed to generate Git push token: {error}"))
    })?;
    let secret = format!("{GIT_PUSH_TOKEN_PREFIX}{}", hex::encode(bytes));
    let token = GitPushToken {
        token_hash: token_hash(&secret),
        owner_user_id: owner_user_id.to_string(),
        created_at_unix: now,
    };

    Ok((secret, token))
}

pub(crate) fn generate_git_clone_token(user_id: &str) -> Result<(String, GitCloneToken), ApiError> {
    let now = unix_now()?;
    let mut bytes = [0_u8; FIRST_PUSH_TOKEN_BYTES];
    getrandom::fill(&mut bytes).map_err(|error| {
        ApiError::internal_message(format!("failed to generate Git clone token: {error}"))
    })?;
    let secret = format!("{GIT_PUSH_TOKEN_PREFIX}{}", hex::encode(bytes));
    let token = GitCloneToken {
        token_hash: token_hash(&secret),
        user_id: user_id.to_string(),
        created_at_unix: now,
    };

    Ok((secret, token))
}

pub(crate) fn token_hash(secret: &str) -> String {
    let digest = Sha256::digest(secret.as_bytes());
    format!("sha256:{digest:x}")
}

pub(crate) fn first_push_token_hash(secret: &str) -> String {
    token_hash(secret)
}

pub(crate) fn git_push_token_hash(secret: &str) -> String {
    token_hash(secret)
}

pub(crate) fn git_clone_token_hash(secret: &str) -> String {
    token_hash(secret)
}
