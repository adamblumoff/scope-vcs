use crate::{config::FIRST_PUSH_TOKEN_BYTES, domain::store::UserAccount, error::ApiError};
use serde::Serialize;

const USER_CODE_BYTES: usize = 8;
pub const MAX_PENDING_DEVICE_LOGINS: u64 = 1024;
pub const MAX_DEVICE_LOGIN_STARTS_PER_WINDOW: u64 = 60;
pub const DEVICE_LOGIN_START_WINDOW_SECS: u64 = 60;
pub const MAX_PENDING_BROWSER_LOGINS: u64 = 1024;
pub const MAX_BROWSER_LOGIN_STARTS_PER_WINDOW: u64 = 60;
pub const BROWSER_LOGIN_START_WINDOW_SECS: u64 = 60;

pub struct DeviceLoginStart {
    pub device_code: String,
    pub user_code: String,
    pub verification_url: String,
    pub expires_at_unix: u64,
    pub poll_interval_secs: u64,
}

pub enum DeviceLoginPoll {
    Pending {
        expires_at_unix: u64,
    },
    Complete {
        session_token: String,
        expires_at_unix: u64,
        identity: SessionIdentity,
    },
}

pub struct BrowserLoginStart {
    pub request_id: String,
    pub request_secret: String,
    pub authorization_url: String,
    pub expires_at_unix: u64,
}

pub struct CliExchangeGrant {
    pub exchange_token: String,
    pub expires_at_unix: u64,
}

pub struct CliSessionToken {
    pub session_token: String,
    pub expires_at_unix: u64,
    pub identity: SessionIdentity,
}

pub struct CliSessionSummary {
    pub id: String,
    pub label: String,
    pub created_at_unix: u64,
    pub last_used_at_unix: Option<u64>,
    pub expires_at_unix: u64,
}

#[derive(Debug, Serialize)]
#[cfg_attr(any(test, feature = "ts"), derive(ts_rs::TS))]
pub struct SessionIdentity {
    pub user_id: String,
    pub email: Option<String>,
    pub email_verified: bool,
}

impl From<&UserAccount> for SessionIdentity {
    fn from(user: &UserAccount) -> Self {
        Self {
            user_id: user.id.clone(),
            email: (!user.email.is_empty()).then(|| user.email.clone()),
            email_verified: user.email_verified,
        }
    }
}

pub fn random_prefixed_token(prefix: &str) -> Result<String, ApiError> {
    let mut bytes = [0_u8; FIRST_PUSH_TOKEN_BYTES];
    getrandom::fill(&mut bytes).map_err(|error| {
        ApiError::internal_message(format!("failed to generate token: {error}"))
    })?;
    Ok(format!("{prefix}{}", hex::encode(bytes)))
}

pub fn random_user_code() -> Result<String, ApiError> {
    let mut bytes = [0_u8; USER_CODE_BYTES];
    getrandom::fill(&mut bytes).map_err(|error| {
        ApiError::internal_message(format!("failed to generate login code: {error}"))
    })?;
    Ok(hex::encode_upper(bytes))
}
