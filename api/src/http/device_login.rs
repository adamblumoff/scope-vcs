use crate::{
    auth::{clerk::bearer_token, scope::require_reconciled_clerk_scope_user},
    config::CLI_SESSION_TOKEN_PREFIX,
    error::ApiError,
    http::{
        origins::public_app_origin,
        responses::{
            DeviceLoginCompleteResponse, DeviceLoginPollResponse, DeviceLoginStartResponse,
            DeviceLoginStatus,
        },
    },
    state::AppState,
};
use axum::{
    Json,
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
};

pub(crate) async fn start_cli_device_login(
    State(state): State<AppState>,
) -> Result<Json<DeviceLoginStartResponse>, ApiError> {
    let app_origin = public_app_origin("build CLI login URL")?;
    let login = state.metadata.start_cli_device_login(&app_origin)?;

    Ok(Json(DeviceLoginStartResponse {
        device_code: login.device_code,
        user_code: login.user_code,
        verification_url: login.verification_url,
        expires_at_unix: login.expires_at_unix,
        poll_interval_secs: login.poll_interval_secs,
    }))
}

pub(crate) async fn complete_cli_device_login(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(user_code): Path<String>,
) -> Result<Json<DeviceLoginCompleteResponse>, ApiError> {
    let user = require_reconciled_clerk_scope_user(&state, &headers).await?;
    state
        .metadata
        .complete_cli_device_login(&user_code, &user)?;

    Ok(Json(DeviceLoginCompleteResponse {
        status: DeviceLoginStatus::Complete,
    }))
}

pub(crate) async fn poll_cli_device_login(
    State(state): State<AppState>,
    Path(device_code): Path<String>,
) -> Result<Json<DeviceLoginPollResponse>, ApiError> {
    match state.metadata.poll_cli_device_login(&device_code)? {
        crate::auth::device::DeviceLoginPoll::Pending { expires_at_unix } => {
            Ok(Json(DeviceLoginPollResponse {
                status: DeviceLoginStatus::Pending,
                session_token: None,
                expires_at_unix,
                identity: None,
            }))
        }
        crate::auth::device::DeviceLoginPoll::Complete {
            session_token,
            expires_at_unix,
            identity,
        } => Ok(Json(DeviceLoginPollResponse {
            status: DeviceLoginStatus::Complete,
            session_token: Some(session_token),
            expires_at_unix,
            identity: Some(identity),
        })),
    }
}

pub(crate) async fn revoke_current_cli_session(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<StatusCode, ApiError> {
    let token =
        bearer_token(&headers)?.ok_or_else(|| ApiError::unauthorized("sign in required"))?;
    if !token.starts_with(CLI_SESSION_TOKEN_PREFIX) {
        return Err(ApiError::unauthorized("CLI session required"));
    }

    state.metadata.revoke_cli_session_token(token)?;
    Ok(StatusCode::NO_CONTENT)
}
