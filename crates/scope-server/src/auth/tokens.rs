use crate::domain::policy::Principal;
use crate::domain::store::{
    AppCatalog, FirstPushToken, GitPushToken, RepoPublicationState, RepoRole, StoredRepository,
};
use crate::{
    config::{
        FIRST_PUSH_TOKEN_BYTES, FIRST_PUSH_TOKEN_PREFIX, FIRST_PUSH_TOKEN_TTL_SECS,
        GIT_PUSH_TOKEN_PREFIX,
    },
    error::ApiError,
    persistence::{lock_catalog, unix_now},
    state::AppState,
};
use sha2::{Digest, Sha256};

pub(crate) fn ensure_owner_setup_access(
    state: &AppState,
    repo: &StoredRepository,
    user_id: &str,
) -> Result<(), ApiError> {
    let catalog = lock_catalog(state)?;
    ensure_owner_setup_access_in_catalog(&catalog, repo, user_id)
}

pub(crate) fn ensure_owner_setup_access_in_catalog(
    catalog: &AppCatalog,
    repo: &StoredRepository,
    user_id: &str,
) -> Result<(), ApiError> {
    let principal = Principal {
        id: user_id.to_string(),
        kind: crate::domain::policy::PrincipalKind::User,
    };
    if catalog.role_for_principal(repo, &principal) != Some(RepoRole::Owner) {
        return Err(ApiError::not_found(format!(
            "repo {} not found",
            repo.record.id
        )));
    }
    if repo.record.publication_state != RepoPublicationState::PendingFirstPush {
        return Err(ApiError::conflict(
            "setup token is only available before the first push",
        ));
    }

    Ok(())
}

pub(crate) fn generate_first_push_token(
    owner_user_id: &str,
) -> Result<(String, FirstPushToken), ApiError> {
    let now = unix_now()?;
    let mut bytes = [0_u8; FIRST_PUSH_TOKEN_BYTES];
    getrandom::fill(&mut bytes).map_err(|error| {
        ApiError::internal_message(format!("failed to generate setup token: {error}"))
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
