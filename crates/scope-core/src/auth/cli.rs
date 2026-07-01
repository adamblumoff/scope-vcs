use super::device::{
    BROWSER_LOGIN_START_WINDOW_SECS, DEVICE_LOGIN_START_WINDOW_SECS,
    MAX_BROWSER_LOGIN_STARTS_PER_WINDOW, MAX_DEVICE_LOGIN_STARTS_PER_WINDOW,
    MAX_PENDING_BROWSER_LOGINS, MAX_PENDING_DEVICE_LOGINS,
};
use crate::error::ApiError;

pub fn enforce_device_login_start_rate_limit(
    pending_count: u64,
    window_count: u64,
) -> Result<(), ApiError> {
    if pending_count >= MAX_PENDING_DEVICE_LOGINS {
        return Err(ApiError::too_many_requests(
            "too many pending CLI device logins",
        ));
    }
    if window_count >= MAX_DEVICE_LOGIN_STARTS_PER_WINDOW {
        return Err(ApiError::too_many_requests(
            "too many CLI device login starts",
        ));
    }
    Ok(())
}

pub fn enforce_browser_login_start_rate_limit(
    pending_count: u64,
    window_count: u64,
) -> Result<(), ApiError> {
    if pending_count >= MAX_PENDING_BROWSER_LOGINS {
        return Err(ApiError::too_many_requests(
            "too many pending CLI browser logins",
        ));
    }
    if window_count >= MAX_BROWSER_LOGIN_STARTS_PER_WINDOW {
        return Err(ApiError::too_many_requests(
            "too many CLI browser login starts",
        ));
    }
    Ok(())
}

pub fn device_login_start_window_start(now: u64) -> u64 {
    now.saturating_sub(DEVICE_LOGIN_START_WINDOW_SECS)
}

pub fn browser_login_start_window_start(now: u64) -> u64 {
    now.saturating_sub(BROWSER_LOGIN_START_WINDOW_SECS)
}

pub struct DeviceLoginCompletionState {
    pub expires_at_unix: u64,
    pub completed: bool,
}

pub enum DeviceLoginCompletionDecision {
    Expired,
    Complete,
}

pub fn decide_device_login_completion(
    state: DeviceLoginCompletionState,
    now: u64,
) -> Result<DeviceLoginCompletionDecision, ApiError> {
    if now >= state.expires_at_unix {
        return Ok(DeviceLoginCompletionDecision::Expired);
    }
    if state.completed {
        return Err(ApiError::conflict("CLI login already completed"));
    }
    Ok(DeviceLoginCompletionDecision::Complete)
}

pub struct DeviceLoginPollState {
    pub expires_at_unix: u64,
    pub consumed: bool,
    pub completed_user_id: Option<String>,
}

pub enum DeviceLoginPollDecision {
    Expired,
    Pending { expires_at_unix: u64 },
    Complete { user_id: String },
}

pub fn decide_device_login_poll(
    state: DeviceLoginPollState,
    now: u64,
) -> Result<DeviceLoginPollDecision, ApiError> {
    if now >= state.expires_at_unix {
        return Ok(DeviceLoginPollDecision::Expired);
    }
    if state.consumed {
        return Err(ApiError::conflict("CLI device login already consumed"));
    }
    let Some(user_id) = state.completed_user_id else {
        return Ok(DeviceLoginPollDecision::Pending {
            expires_at_unix: state.expires_at_unix,
        });
    };
    Ok(DeviceLoginPollDecision::Complete { user_id })
}

pub struct BrowserLoginCompletionState {
    pub expires_at_unix: u64,
    pub consumed: bool,
    pub completed: bool,
}

pub enum BrowserLoginCompletionDecision {
    Expired,
    Complete,
}

pub fn decide_browser_login_completion(
    state: BrowserLoginCompletionState,
    now: u64,
) -> Result<BrowserLoginCompletionDecision, ApiError> {
    if now >= state.expires_at_unix {
        return Ok(BrowserLoginCompletionDecision::Expired);
    }
    if state.consumed {
        return Err(ApiError::conflict("CLI browser login already consumed"));
    }
    if state.completed {
        return Err(ApiError::conflict("CLI browser login already completed"));
    }
    Ok(BrowserLoginCompletionDecision::Complete)
}

pub struct BrowserLoginExchangeState {
    pub expires_at_unix: u64,
    pub consumed: bool,
    pub request_secret_hash: String,
    pub callback_code_hash: Option<String>,
    pub completed_user_id: Option<String>,
}

pub enum BrowserLoginExchangeDecision {
    Expired,
    Complete { user_id: String },
}

pub fn decide_browser_login_exchange(
    state: BrowserLoginExchangeState,
    now: u64,
    request_secret_hash: &str,
    callback_code_hash: &str,
) -> Result<BrowserLoginExchangeDecision, ApiError> {
    if now >= state.expires_at_unix {
        return Ok(BrowserLoginExchangeDecision::Expired);
    }
    if state.consumed {
        return Err(ApiError::conflict("CLI browser login already consumed"));
    }
    if state.request_secret_hash != request_secret_hash {
        return Err(ApiError::unauthorized("invalid CLI browser login secret"));
    }
    if state.callback_code_hash.as_deref() != Some(callback_code_hash) {
        return Err(ApiError::unauthorized("invalid CLI browser login code"));
    }
    let Some(user_id) = state.completed_user_id else {
        return Err(ApiError::conflict("CLI browser login is pending"));
    };
    Ok(BrowserLoginExchangeDecision::Complete { user_id })
}

pub struct CliExchangeGrantState {
    pub expires_at_unix: u64,
    pub consumed: bool,
    pub user_id: String,
}

pub enum CliExchangeGrantDecision {
    Expired,
    Complete { user_id: String },
}

pub fn decide_cli_exchange_grant(
    state: CliExchangeGrantState,
    now: u64,
) -> Result<CliExchangeGrantDecision, ApiError> {
    if now >= state.expires_at_unix {
        return Ok(CliExchangeGrantDecision::Expired);
    }
    if state.consumed {
        return Err(ApiError::conflict("CLI exchange token already used"));
    }
    Ok(CliExchangeGrantDecision::Complete {
        user_id: state.user_id,
    })
}

pub struct CliSessionState {
    pub expires_at_unix: u64,
    pub revoked: bool,
    pub user_id: String,
}

pub enum CliSessionUseDecision {
    Expired,
    Active { user_id: String },
}

pub fn decide_cli_session_use(
    state: CliSessionState,
    now: u64,
) -> Result<CliSessionUseDecision, ApiError> {
    if now >= state.expires_at_unix {
        return Ok(CliSessionUseDecision::Expired);
    }
    if state.revoked {
        return Err(ApiError::unauthorized("CLI session revoked"));
    }
    Ok(CliSessionUseDecision::Active {
        user_id: state.user_id,
    })
}

pub enum CliSessionRevokeDecision {
    Expired,
    Revoke,
}

pub fn decide_cli_session_revoke(expires_at_unix: u64, now: u64) -> CliSessionRevokeDecision {
    if now >= expires_at_unix {
        return CliSessionRevokeDecision::Expired;
    }
    CliSessionRevokeDecision::Revoke
}
