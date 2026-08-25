use crate::{
    auth::{
        clerk::{ClerkIdentity, bearer_token},
        cli::CliAuthService,
    },
    config::CLI_SESSION_TOKEN_PREFIX,
    error::ApiError,
    persistence::unix_now,
    product_analytics::ProductEvent,
    state::AppState,
};
use axum::http::HeaderMap;
use scope_domain::{
    account::UserAccount,
    policy::{Principal, PrincipalKind},
    repository::Repository,
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
    reconcile_clerk_scope_user(state, &identity).await
}

async fn reconcile_clerk_scope_user(
    state: &AppState,
    identity: &ClerkIdentity,
) -> Result<UserAccount, ApiError> {
    let resolution = state
        .metadata
        .auth()
        .resolve_clerk_user(identity, unix_now()?)
        .await?;
    if resolution.created {
        state
            .product_analytics
            .capture(ProductEvent::account_created(&resolution.user.id));
    }
    Ok(resolution.user)
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
        None => reconcile_clerk_scope_user(state, identity).await,
    }
}

pub(crate) fn principal_for_scope_user(repo: &Repository, user: Option<&UserAccount>) -> Principal {
    let Some(user) = user else {
        return Principal::public();
    };
    principal_for_user_id(repo, &user.id)
}

pub(crate) fn principal_for_user_id(repo: &Repository, user_id: &str) -> Principal {
    if repo.is_owner_user(user_id) || repo.member_for_user(user_id).is_some() {
        Principal {
            id: user_id.to_string(),
            kind: PrincipalKind::User,
        }
    } else {
        Principal::public()
    }
}
