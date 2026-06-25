use crate::{
    auth::clerk::{bearer_token, ensure_user_for_identity},
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
    http::HeaderMap,
};

pub(crate) async fn start_cli_device_login(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<DeviceLoginStartResponse>, ApiError> {
    let app_origin = public_app_origin("build CLI login URL")?;
    let login = state
        .device_logins
        .start(&app_origin, &device_login_client_key(&headers))?;

    Ok(Json(DeviceLoginStartResponse {
        device_code: login.device_code,
        user_code: login.user_code,
        verification_url: login.verification_url,
        expires_at_unix: login.expires_at_unix,
        poll_interval_secs: login.poll_interval_secs,
    }))
}

fn device_login_client_key(headers: &HeaderMap) -> String {
    for name in ["x-forwarded-for", "x-real-ip", "cf-connecting-ip"] {
        if let Some(value) = headers.get(name).and_then(|value| value.to_str().ok()) {
            let first = value.split(',').next().unwrap_or_default().trim();
            if !first.is_empty() {
                return first.to_string();
            }
        }
    }

    "unknown".to_string()
}

pub(crate) async fn complete_cli_device_login(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(user_code): Path<String>,
) -> Result<Json<DeviceLoginCompleteResponse>, ApiError> {
    let token =
        bearer_token(&headers)?.ok_or_else(|| ApiError::unauthorized("sign in required"))?;
    let identity = state.clerk.verify(token).await?;
    ensure_user_for_identity(&state, &identity)?;
    state.device_logins.complete(&user_code, identity)?;

    Ok(Json(DeviceLoginCompleteResponse {
        status: DeviceLoginStatus::Complete,
    }))
}

pub(crate) async fn poll_cli_device_login(
    State(state): State<AppState>,
    Path(device_code): Path<String>,
) -> Result<Json<DeviceLoginPollResponse>, ApiError> {
    match state.device_logins.poll(&device_code)? {
        crate::auth::device::DeviceLoginPoll::Pending { expires_at_unix } => {
            Ok(Json(DeviceLoginPollResponse {
                status: DeviceLoginStatus::Pending,
                access_token: None,
                expires_at_unix,
                identity: None,
            }))
        }
        crate::auth::device::DeviceLoginPoll::Complete {
            access_token,
            expires_at_unix,
            identity,
        } => Ok(Json(DeviceLoginPollResponse {
            status: DeviceLoginStatus::Complete,
            access_token: Some(access_token),
            expires_at_unix,
            identity: Some(identity),
        })),
    }
}
