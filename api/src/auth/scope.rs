use crate::{
    auth::clerk::{ClerkIdentity, bearer_token},
    config::CLI_SESSION_TOKEN_PREFIX,
    domain::{
        policy::{Principal, PrincipalKind},
        store::{StoredRepository, UserAccount},
    },
    error::ApiError,
    state::AppState,
};
use axum::http::HeaderMap;

pub(crate) async fn optional_scope_user(
    state: &AppState,
    headers: &HeaderMap,
) -> Result<Option<UserAccount>, ApiError> {
    let Some(token) = bearer_token(headers)? else {
        return Ok(None);
    };

    if token.starts_with(CLI_SESSION_TOKEN_PREFIX) {
        return state.metadata.verify_cli_session_token(token).map(Some);
    }

    let identity = state.clerk.verify(token).await?;
    state.metadata.resolve_clerk_user(&identity).map(Some)
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
    state.metadata.resolve_clerk_user(&identity)
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
    if repo
        .memberships
        .iter()
        .any(|membership| membership.user_id == user_id)
    {
        Principal {
            id: user_id.to_string(),
            kind: PrincipalKind::User,
        }
    } else {
        Principal::public()
    }
}
