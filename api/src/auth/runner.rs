use crate::{
    auth::{clerk::bearer_token, tokens::machine_token_hash},
    error::ApiError,
    persistence::unix_now,
    state::AppState,
};
use axum::http::HeaderMap;
use scope_domain::runs::runner::Runner;
use scope_postgres::db::DispatchClaim;

pub(crate) async fn require_runner(
    state: &AppState,
    headers: &HeaderMap,
) -> Result<Runner, ApiError> {
    let secret =
        bearer_token(headers)?.ok_or_else(|| ApiError::unauthorized("runner token required"))?;
    if !secret.starts_with("scope_runner_") {
        return Err(ApiError::unauthorized("runner credentials are invalid"));
    }
    Ok(state
        .metadata
        .runs()
        .authenticate_runner(&machine_token_hash(secret), unix_now()?)
        .await?)
}

pub(crate) async fn require_attempt(
    state: &AppState,
    headers: &HeaderMap,
    attempt_id: &str,
) -> Result<DispatchClaim, ApiError> {
    let secret =
        bearer_token(headers)?.ok_or_else(|| ApiError::unauthorized("attempt token required"))?;
    if !secret.starts_with("scope_attempt_") {
        return Err(ApiError::unauthorized("attempt credentials are invalid"));
    }
    Ok(state
        .metadata
        .runs()
        .authenticate_attempt(attempt_id, &machine_token_hash(secret), unix_now()?)
        .await?)
}

pub(crate) fn attempt_token_hash(headers: &HeaderMap) -> Result<String, ApiError> {
    let secret =
        bearer_token(headers)?.ok_or_else(|| ApiError::unauthorized("attempt token required"))?;
    if !secret.starts_with("scope_attempt_") {
        return Err(ApiError::unauthorized("attempt credentials are invalid"));
    }
    Ok(machine_token_hash(secret))
}
