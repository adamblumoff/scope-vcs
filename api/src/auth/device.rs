use crate::{
    auth::{clerk::ClerkIdentity, tokens::token_hash},
    config::{
        CLI_ACCESS_TOKEN_PREFIX, CLI_ACCESS_TOKEN_TTL_SECS, CLI_DEVICE_CODE_PREFIX,
        CLI_DEVICE_LOGIN_POLL_INTERVAL_SECS, CLI_DEVICE_LOGIN_TTL_SECS, FIRST_PUSH_TOKEN_BYTES,
    },
    error::ApiError,
    http::responses::SessionIdentity,
    persistence::unix_now,
};
use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex},
};

const USER_CODE_BYTES: usize = 16;
pub(crate) const MAX_PENDING_DEVICE_LOGINS: usize = 1024;
pub(crate) const MAX_PENDING_DEVICE_LOGINS_PER_CLIENT: usize = 3;
const MAX_FAILED_COMPLETION_ATTEMPTS: u32 = 10;
const COMPLETION_ATTEMPT_WINDOW_SECS: u64 = 60;

#[derive(Clone, Default)]
pub(crate) struct DeviceLoginStore {
    state: Arc<Mutex<DeviceLoginState>>,
}

#[derive(Default)]
struct DeviceLoginState {
    logins_by_device_code: BTreeMap<String, DeviceLoginEntry>,
    device_code_by_user_code: BTreeMap<String, String>,
    pending_count_by_client_key: BTreeMap<String, usize>,
    access_sessions_by_token_hash: BTreeMap<String, CliAccessSession>,
    completion_attempts_by_user_id: BTreeMap<String, CompletionAttemptWindow>,
}

struct DeviceLoginEntry {
    user_code: String,
    client_key: String,
    expires_at_unix: u64,
    completed: Option<CompletedDeviceLogin>,
}

struct CompletedDeviceLogin {
    identity: ClerkIdentity,
    access_token: String,
    access_expires_at_unix: u64,
}

struct CompletionAttemptWindow {
    failures: u32,
    reset_at_unix: u64,
}

#[derive(Clone)]
struct CliAccessSession {
    identity: ClerkIdentity,
    expires_at_unix: u64,
}

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
        access_token: String,
        expires_at_unix: u64,
        identity: SessionIdentity,
    },
}

impl DeviceLoginStore {
    pub(crate) fn start(
        &self,
        app_origin: &str,
        client_key: &str,
    ) -> Result<DeviceLoginStart, ApiError> {
        let now = unix_now()?;
        let expires_at_unix = now + CLI_DEVICE_LOGIN_TTL_SECS;
        let mut state = self.lock_state();
        state.cleanup_expired(now);
        let client_key = normalize_client_key(client_key);
        if state
            .pending_count_by_client_key
            .get(&client_key)
            .copied()
            .unwrap_or_default()
            >= MAX_PENDING_DEVICE_LOGINS_PER_CLIENT
        {
            return Err(ApiError::too_many_requests(
                "too many pending CLI device logins for this client",
            ));
        }
        if state.logins_by_device_code.len() >= MAX_PENDING_DEVICE_LOGINS {
            return Err(ApiError::too_many_requests(
                "too many pending CLI device logins",
            ));
        }

        let (device_code, user_code) = loop {
            let device_code = random_prefixed_token(CLI_DEVICE_CODE_PREFIX)?;
            let user_code = random_user_code()?;
            if !state.logins_by_device_code.contains_key(&device_code)
                && !state.device_code_by_user_code.contains_key(&user_code)
            {
                break (device_code, user_code);
            }
        };
        let verification_url = format!(
            "{}/cli-login?code={}",
            app_origin.trim_end_matches('/'),
            user_code
        );
        let entry = DeviceLoginEntry {
            user_code: user_code.clone(),
            client_key: client_key.clone(),
            expires_at_unix,
            completed: None,
        };

        state
            .device_code_by_user_code
            .insert(user_code.clone(), device_code.clone());
        state
            .logins_by_device_code
            .insert(device_code.clone(), entry);
        *state
            .pending_count_by_client_key
            .entry(client_key)
            .or_default() += 1;

        Ok(DeviceLoginStart {
            device_code,
            user_code,
            verification_url,
            expires_at_unix,
            poll_interval_secs: CLI_DEVICE_LOGIN_POLL_INTERVAL_SECS,
        })
    }

    pub(crate) fn complete(
        &self,
        raw_user_code: &str,
        identity: ClerkIdentity,
    ) -> Result<(), ApiError> {
        let now = unix_now()?;
        let mut state = self.lock_state();
        state.cleanup_expired(now);
        let identity_user_id = identity.user_id.clone();
        state.ensure_completion_attempt_allowed(&identity_user_id, now)?;

        let user_code = normalize_user_code(raw_user_code);
        let Some(device_code) = state.device_code_by_user_code.get(&user_code).cloned() else {
            state.record_completion_failure(&identity_user_id, now);
            return Err(ApiError::not_found("CLI login code not found"));
        };
        let Some(entry) = state.logins_by_device_code.get(&device_code) else {
            state.record_completion_failure(&identity_user_id, now);
            return Err(ApiError::not_found("CLI login code not found"));
        };
        let expires_at_unix = entry.expires_at_unix;

        if now >= expires_at_unix {
            state.remove_login(&device_code);
            state.record_completion_failure(&identity_user_id, now);
            return Err(ApiError::conflict("CLI login code expired"));
        }
        if entry.completed.is_some() {
            return Err(ApiError::conflict("CLI login already completed"));
        }

        let access_token = random_prefixed_token(CLI_ACCESS_TOKEN_PREFIX)?;
        let access_expires_at_unix = now + CLI_ACCESS_TOKEN_TTL_SECS;
        state.access_sessions_by_token_hash.insert(
            token_hash(&access_token),
            CliAccessSession {
                identity: identity.clone(),
                expires_at_unix: access_expires_at_unix,
            },
        );
        state
            .logins_by_device_code
            .get_mut(&device_code)
            .expect("device login must exist after expiration check")
            .completed = Some(CompletedDeviceLogin {
            identity,
            access_token,
            access_expires_at_unix,
        });
        state.clear_completion_failures(&identity_user_id);

        Ok(())
    }

    pub(crate) fn poll(&self, device_code: &str) -> Result<DeviceLoginPoll, ApiError> {
        let now = unix_now()?;
        let mut state = self.lock_state();
        state.cleanup_expired(now);

        let Some(entry) = state.logins_by_device_code.get(device_code) else {
            return Err(ApiError::not_found("CLI device login not found"));
        };
        if now >= entry.expires_at_unix {
            state.remove_login(device_code);
            return Err(ApiError::conflict("CLI device login expired"));
        }
        if entry.completed.is_none() {
            return Ok(DeviceLoginPoll::Pending {
                expires_at_unix: entry.expires_at_unix,
            });
        }

        let entry = state
            .remove_login(device_code)
            .expect("device login must exist after lookup");
        let completed = entry
            .completed
            .expect("completed device login must include access token");

        Ok(DeviceLoginPoll::Complete {
            access_token: completed.access_token,
            expires_at_unix: completed.access_expires_at_unix,
            identity: SessionIdentity::from(&completed.identity),
        })
    }

    pub(crate) fn verify_access_token(
        &self,
        access_token: &str,
    ) -> Result<ClerkIdentity, ApiError> {
        let now = unix_now()?;
        let mut state = self.lock_state();
        state.cleanup_expired(now);
        let session = state
            .access_sessions_by_token_hash
            .get(&token_hash(access_token))
            .ok_or_else(|| ApiError::unauthorized("invalid CLI token"))?;

        if now >= session.expires_at_unix {
            return Err(ApiError::unauthorized("CLI token expired"));
        }

        Ok(session.identity.clone())
    }

    fn lock_state(&self) -> std::sync::MutexGuard<'_, DeviceLoginState> {
        self.state
            .lock()
            .expect("CLI device login store lock must not be poisoned")
    }
}

impl DeviceLoginState {
    fn cleanup_expired(&mut self, now: u64) {
        let expired_device_codes = self
            .logins_by_device_code
            .iter()
            .filter_map(|(device_code, entry)| {
                (now >= entry.expires_at_unix).then(|| device_code.clone())
            })
            .collect::<Vec<_>>();
        for device_code in expired_device_codes {
            self.remove_login(&device_code);
        }

        self.access_sessions_by_token_hash
            .retain(|_, session| now < session.expires_at_unix);
        self.completion_attempts_by_user_id
            .retain(|_, window| now < window.reset_at_unix);
    }

    fn remove_login(&mut self, device_code: &str) -> Option<DeviceLoginEntry> {
        let entry = self.logins_by_device_code.remove(device_code)?;
        self.device_code_by_user_code.remove(&entry.user_code);
        decrement_pending_client_count(&mut self.pending_count_by_client_key, &entry.client_key);
        Some(entry)
    }

    fn ensure_completion_attempt_allowed(
        &mut self,
        user_id: &str,
        now: u64,
    ) -> Result<(), ApiError> {
        self.completion_attempts_by_user_id
            .retain(|_, window| now < window.reset_at_unix);
        if self
            .completion_attempts_by_user_id
            .get(user_id)
            .is_some_and(|window| window.failures >= MAX_FAILED_COMPLETION_ATTEMPTS)
        {
            return Err(ApiError::too_many_requests(
                "too many CLI login completion attempts",
            ));
        }
        Ok(())
    }

    fn record_completion_failure(&mut self, user_id: &str, now: u64) {
        let window = self
            .completion_attempts_by_user_id
            .entry(user_id.to_string())
            .or_insert(CompletionAttemptWindow {
                failures: 0,
                reset_at_unix: now + COMPLETION_ATTEMPT_WINDOW_SECS,
            });
        if now >= window.reset_at_unix {
            window.failures = 0;
            window.reset_at_unix = now + COMPLETION_ATTEMPT_WINDOW_SECS;
        }
        window.failures = window.failures.saturating_add(1);
    }

    fn clear_completion_failures(&mut self, user_id: &str) {
        self.completion_attempts_by_user_id.remove(user_id);
    }
}

fn random_prefixed_token(prefix: &str) -> Result<String, ApiError> {
    let mut bytes = [0_u8; FIRST_PUSH_TOKEN_BYTES];
    getrandom::fill(&mut bytes).map_err(|error| {
        ApiError::internal_message(format!("failed to generate token: {error}"))
    })?;
    Ok(format!("{prefix}{}", hex::encode(bytes)))
}

fn random_user_code() -> Result<String, ApiError> {
    let mut bytes = [0_u8; USER_CODE_BYTES];
    getrandom::fill(&mut bytes).map_err(|error| {
        ApiError::internal_message(format!("failed to generate login code: {error}"))
    })?;
    Ok(hex::encode_upper(bytes))
}

fn normalize_client_key(value: &str) -> String {
    let value = value.trim();
    if value.is_empty() {
        "unknown".to_string()
    } else {
        value.to_string()
    }
}

fn decrement_pending_client_count(counts: &mut BTreeMap<String, usize>, client_key: &str) {
    let Some(count) = counts.get_mut(client_key) else {
        return;
    };
    *count = count.saturating_sub(1);
    if *count == 0 {
        counts.remove(client_key);
    }
}

fn normalize_user_code(value: &str) -> String {
    value
        .trim()
        .chars()
        .filter(|ch| !ch.is_whitespace() && *ch != '-')
        .flat_map(char::to_uppercase)
        .collect()
}
