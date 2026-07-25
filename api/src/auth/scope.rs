use crate::{
    auth::{
        clerk::{ClerkIdentity, bearer_token},
        cli::CliAuthService,
    },
    config::CLI_SESSION_TOKEN_PREFIX,
    error::ApiError,
    persistence::unix_now,
    state::AppState,
};
use axum::http::HeaderMap;
use scope_domain::{
    policy::{Principal, PrincipalKind},
    store::{StoredRepository, UserAccount},
};

pub(crate) async fn optional_scope_user(
    state: &AppState,
    headers: &HeaderMap,
) -> Result<Option<UserAccount>, ApiError> {
    let Some(token) = bearer_token(headers)? else {
        return Ok(None);
    };

    if token.starts_with(CLI_SESSION_TOKEN_PREFIX) {
        return CliAuthService::new(state.metadata.auth())
            .verify_session_token(token, unix_now()?)
            .await
            .map(Some);
    }

    let identity = state.clerk.verify(token).await?;
    resolve_clerk_scope_user(state, &identity).await.map(Some)
}

pub(crate) async fn require_scope_user(
    state: &AppState,
    headers: &HeaderMap,
) -> Result<UserAccount, ApiError> {
    optional_scope_user(state, headers)
        .await?
        .ok_or_else(|| ApiError::unauthorized("sign in required"))
}

pub(crate) async fn require_clerk_scope_user(
    state: &AppState,
    headers: &HeaderMap,
) -> Result<UserAccount, ApiError> {
    let identity = require_clerk_identity(state, headers).await?;
    resolve_clerk_scope_user(state, &identity).await
}

pub(crate) async fn require_reconciled_clerk_scope_user(
    state: &AppState,
    headers: &HeaderMap,
) -> Result<UserAccount, ApiError> {
    let identity = require_clerk_identity(state, headers).await?;
    Ok(state
        .metadata
        .auth()
        .resolve_clerk_user(&identity, unix_now()?)
        .await?)
}

pub(crate) async fn require_clerk_identity(
    state: &AppState,
    headers: &HeaderMap,
) -> Result<ClerkIdentity, ApiError> {
    let token = bearer_token(headers)?.ok_or_else(|| ApiError::unauthorized("sign in required"))?;
    if token.starts_with(CLI_SESSION_TOKEN_PREFIX) {
        return Err(ApiError::unauthorized("Clerk auth required"));
    }
    state.clerk.verify(token).await
}

async fn resolve_clerk_scope_user(
    state: &AppState,
    identity: &ClerkIdentity,
) -> Result<UserAccount, ApiError> {
    match state
        .metadata
        .auth()
        .resolve_existing_clerk_user(identity)
        .await?
    {
        Some(user) => Ok(user),
        None => Ok(state
            .metadata
            .auth()
            .resolve_clerk_user(identity, unix_now()?)
            .await?),
    }
}

pub(crate) fn principal_for_scope_user(
    repo: &StoredRepository,
    user: Option<&UserAccount>,
) -> Principal {
    let Some(user) = user else {
        return Principal::public();
    };
    principal_for_user_id(repo, &user.id)
}

pub(crate) fn principal_for_user_id(repo: &StoredRepository, user_id: &str) -> Principal {
    if repo.is_owner_user(user_id) || repo.member_for_user(user_id).is_some() {
        Principal {
            id: user_id.to_string(),
            kind: PrincipalKind::User,
        }
    } else {
        Principal::public()
    }
}
