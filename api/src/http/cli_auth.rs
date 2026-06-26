use crate::{
    auth::{device::CliSessionSummary, scope::require_clerk_scope_user},
    error::ApiError,
    http::{
        origins::public_app_origin,
        responses::{
            BrowserLoginCompleteResponse, BrowserLoginExchangeRequest, BrowserLoginStartRequest,
            BrowserLoginStartResponse, CliExchangeGrantExchangeRequest, CliExchangeGrantResponse,
            CliSessionResponse, CliSessionTokenResponse, CliSessionsResponse,
        },
    },
    state::AppState,
};
use axum::{
    Json,
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
};

pub(crate) async fn start_cli_browser_login(
    State(state): State<AppState>,
    Json(request): Json<BrowserLoginStartRequest>,
) -> Result<Json<BrowserLoginStartResponse>, ApiError> {
    let app_origin = public_app_origin("build CLI browser login URL")?;
    let login = state
        .metadata
        .start_cli_browser_login(&app_origin, &request.callback_url)?;

    Ok(Json(BrowserLoginStartResponse {
        request_id: login.request_id,
        request_secret: login.request_secret,
        authorization_url: login.authorization_url,
        expires_at_unix: login.expires_at_unix,
    }))
}

pub(crate) async fn complete_cli_browser_login(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(request_id): Path<String>,
) -> Result<Json<BrowserLoginCompleteResponse>, ApiError> {
    let user = require_clerk_scope_user(&state, &headers).await?;
    let redirect_url = state
        .metadata
        .complete_cli_browser_login(&request_id, &user)?;

    Ok(Json(BrowserLoginCompleteResponse { redirect_url }))
}

pub(crate) async fn exchange_cli_browser_login(
    State(state): State<AppState>,
    Path(request_id): Path<String>,
    Json(request): Json<BrowserLoginExchangeRequest>,
) -> Result<Json<CliSessionTokenResponse>, ApiError> {
    let token = state.metadata.exchange_cli_browser_login(
        &request_id,
        &request.request_secret,
        &request.callback_code,
    )?;

    Ok(Json(CliSessionTokenResponse {
        session_token: token.session_token,
        expires_at_unix: token.expires_at_unix,
        identity: token.identity,
    }))
}

pub(crate) async fn create_cli_exchange_grant(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<CliExchangeGrantResponse>, ApiError> {
    let user = require_clerk_scope_user(&state, &headers).await?;
    let grant = state.metadata.create_cli_exchange_grant(&user)?;

    Ok(Json(CliExchangeGrantResponse {
        exchange_token: grant.exchange_token,
        expires_at_unix: grant.expires_at_unix,
    }))
}

pub(crate) async fn exchange_cli_grant(
    State(state): State<AppState>,
    Json(request): Json<CliExchangeGrantExchangeRequest>,
) -> Result<Json<CliSessionTokenResponse>, ApiError> {
    let token = state.metadata.exchange_cli_grant(&request.exchange_token)?;

    Ok(Json(CliSessionTokenResponse {
        session_token: token.session_token,
        expires_at_unix: token.expires_at_unix,
        identity: token.identity,
    }))
}

pub(crate) async fn list_cli_sessions(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<CliSessionsResponse>, ApiError> {
    let user = require_clerk_scope_user(&state, &headers).await?;
    let sessions = state
        .metadata
        .list_cli_sessions_for_user(&user)?
        .into_iter()
        .map(cli_session_response)
        .collect();

    Ok(Json(CliSessionsResponse { sessions }))
}

pub(crate) async fn revoke_cli_session(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(session_id): Path<String>,
) -> Result<StatusCode, ApiError> {
    let user = require_clerk_scope_user(&state, &headers).await?;
    state
        .metadata
        .revoke_cli_session_for_user(&user, &session_id)?;
    Ok(StatusCode::NO_CONTENT)
}

fn cli_session_response(session: CliSessionSummary) -> CliSessionResponse {
    CliSessionResponse {
        id: session.id,
        label: session.label,
        created_at_unix: session.created_at_unix,
        last_used_at_unix: session.last_used_at_unix,
        expires_at_unix: session.expires_at_unix,
    }
}
