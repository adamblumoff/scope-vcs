use crate::{config::FIRST_PUSH_TOKEN_BYTES, error::ApiError, http::responses::SessionIdentity};

const USER_CODE_BYTES: usize = 8;
pub(crate) const MAX_PENDING_DEVICE_LOGINS: u64 = 1024;
pub(crate) const MAX_DEVICE_LOGIN_STARTS_PER_WINDOW: u64 = 60;
pub(crate) const DEVICE_LOGIN_START_WINDOW_SECS: u64 = 60;

pub(crate) struct DeviceLoginStart {
    pub(crate) device_code: String,
    pub(crate) user_code: String,
    pub(crate) verification_url: String,
    pub(crate) expires_at_unix: u64,
    pub(crate) poll_interval_secs: u64,
}

pub(crate) enum DeviceLoginPoll {
    Pending {
        expires_at_unix: u64,
    },
    Complete {
        session_token: String,
        expires_at_unix: u64,
        identity: SessionIdentity,
    },
}

pub(crate) struct BrowserLoginStart {
    pub(crate) request_id: String,
    pub(crate) request_secret: String,
    pub(crate) authorization_url: String,
    pub(crate) expires_at_unix: u64,
}

pub(crate) struct CliExchangeGrant {
    pub(crate) exchange_token: String,
    pub(crate) expires_at_unix: u64,
}

pub(crate) struct CliSessionToken {
    pub(crate) session_token: String,
    pub(crate) expires_at_unix: u64,
    pub(crate) identity: SessionIdentity,
}

pub(crate) struct CliSessionSummary {
    pub(crate) id: String,
    pub(crate) label: String,
    pub(crate) created_at_unix: u64,
    pub(crate) last_used_at_unix: Option<u64>,
    pub(crate) expires_at_unix: u64,
}

pub(crate) fn random_prefixed_token(prefix: &str) -> Result<String, ApiError> {
    let mut bytes = [0_u8; FIRST_PUSH_TOKEN_BYTES];
    getrandom::fill(&mut bytes).map_err(|error| {
        ApiError::internal_message(format!("failed to generate token: {error}"))
    })?;
    Ok(format!("{prefix}{}", hex::encode(bytes)))
}

pub(crate) fn random_user_code() -> Result<String, ApiError> {
    let mut bytes = [0_u8; USER_CODE_BYTES];
    getrandom::fill(&mut bytes).map_err(|error| {
        ApiError::internal_message(format!("failed to generate login code: {error}"))
    })?;
    Ok(hex::encode_upper(bytes))
}
