use super::tokens::{random_token, token_hash};
use crate::{config::CLI_SESSION_TOKEN_PREFIX, error::ApiError};
use scope_domain::{account::SessionIdentity, account::UserAccount};
use scope_postgres::db::{
    AuthStore, CreateCliExchangeGrantCommand, DeviceLoginPoll as PersistedDeviceLoginPoll,
    NewCliSession, StartBrowserLoginCommand, StartDeviceLoginCommand,
};
use url::Url;

const CLI_BROWSER_LOGIN_ID_PREFIX: &str = "cli_browser_";
const CLI_BROWSER_LOGIN_SECRET_PREFIX: &str = "scope_browser_";
const CLI_CALLBACK_CODE_PREFIX: &str = "scope_callback_";
const CLI_EXCHANGE_GRANT_PREFIX: &str = "scope_otc_";
const CLI_SESSION_ID_PREFIX: &str = "cli_sess_";
const CLI_DEVICE_CODE_PREFIX: &str = "scope_device_";
const CLI_BROWSER_LOGIN_TTL_SECS: u64 = 5 * 60;
const CLI_DEVICE_LOGIN_TTL_SECS: u64 = 10 * 60;
const CLI_EXCHANGE_GRANT_TTL_SECS: u64 = 5 * 60;
const CLI_SESSION_TTL_SECS: u64 = 30 * 24 * 60 * 60;
const CLI_DEVICE_LOGIN_POLL_INTERVAL_SECS: u64 = 2;
const USER_CODE_BYTES: usize = 8;

pub(crate) struct CliAuthService {
    store: AuthStore,
}

pub(crate) struct DeviceLoginStart {
    pub device_code: String,
    pub user_code: String,
    pub verification_url: String,
    pub expires_at_unix: u64,
    pub poll_interval_secs: u64,
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
    pub request_id: String,
    pub request_secret: String,
    pub authorization_url: String,
    pub expires_at_unix: u64,
}

pub(crate) struct CliExchangeGrant {
    pub exchange_token: String,
    pub expires_at_unix: u64,
}

pub(crate) struct CliSessionToken {
    pub session_token: String,
    pub expires_at_unix: u64,
    pub identity: SessionIdentity,
}

impl CliAuthService {
    pub(crate) fn new(store: AuthStore) -> Self {
        Self { store }
    }

    pub(crate) async fn start_device_login(
        &self,
        app_origin: &str,
        now_unix: u64,
    ) -> Result<DeviceLoginStart, ApiError> {
        let device_code =
            random_token(CLI_DEVICE_CODE_PREFIX, "failed to generate CLI device code")?;
        let user_code = random_user_code()?;
        let expires_at_unix = now_unix + CLI_DEVICE_LOGIN_TTL_SECS;
        self.store
            .start_cli_device_login(
                StartDeviceLoginCommand {
                    device_code_hash: token_hash(&device_code),
                    user_code_hash: token_hash(&normalize_user_code(&user_code)),
                    created_at_unix: now_unix,
                    expires_at_unix,
                },
                now_unix,
            )
            .await?;
        Ok(DeviceLoginStart {
            device_code,
            user_code,
            verification_url: format!("{}/cli-login", app_origin.trim_end_matches('/')),
            expires_at_unix,
            poll_interval_secs: CLI_DEVICE_LOGIN_POLL_INTERVAL_SECS,
        })
    }

    pub(crate) async fn complete_device_login(
        &self,
        user_code: &str,
        user: &UserAccount,
        now_unix: u64,
    ) -> Result<(), ApiError> {
        self.store
            .complete_cli_device_login_by_user_code_hash(
                &token_hash(&normalize_user_code(user_code)),
                user,
                now_unix,
            )
            .await?;
        Ok(())
    }

    pub(crate) async fn poll_device_login(
        &self,
        device_code: &str,
        now_unix: u64,
    ) -> Result<DeviceLoginPoll, ApiError> {
        let (session_token, session) = new_cli_session(now_unix)?;
        let expires_at_unix = session.expires_at_unix;
        match self
            .store
            .poll_cli_device_login_by_hash(&token_hash(device_code), session, now_unix)
            .await?
        {
            PersistedDeviceLoginPoll::Pending { expires_at_unix } => {
                Ok(DeviceLoginPoll::Pending { expires_at_unix })
            }
            PersistedDeviceLoginPoll::Complete { identity } => Ok(DeviceLoginPoll::Complete {
                session_token,
                expires_at_unix,
                identity,
            }),
        }
    }

    pub(crate) async fn verify_session_token(
        &self,
        session_token: &str,
        now_unix: u64,
    ) -> Result<UserAccount, ApiError> {
        Ok(self
            .store
            .verify_cli_session_by_hash(&token_hash(session_token), now_unix)
            .await?)
    }

    pub(crate) async fn revoke_session_token(
        &self,
        session_token: &str,
        now_unix: u64,
    ) -> Result<(), ApiError> {
        self.store
            .revoke_cli_session_by_hash(&token_hash(session_token), now_unix)
            .await?;
        Ok(())
    }

    pub(crate) async fn start_browser_login(
        &self,
        app_origin: &str,
        callback_url: &str,
        now_unix: u64,
    ) -> Result<BrowserLoginStart, ApiError> {
        validate_loopback_callback_url(callback_url)?;
        let request_id = random_token(
            CLI_BROWSER_LOGIN_ID_PREFIX,
            "failed to generate CLI browser request ID",
        )?;
        let request_secret = random_token(
            CLI_BROWSER_LOGIN_SECRET_PREFIX,
            "failed to generate CLI browser request secret",
        )?;
        let expires_at_unix = now_unix + CLI_BROWSER_LOGIN_TTL_SECS;
        let authorization_url = browser_authorization_url(app_origin, &request_id)?;
        self.store
            .start_cli_browser_login(
                StartBrowserLoginCommand {
                    request_id: request_id.clone(),
                    request_secret_hash: token_hash(&request_secret),
                    callback_url: callback_url.to_string(),
                    created_at_unix: now_unix,
                    expires_at_unix,
                },
                now_unix,
            )
            .await?;
        Ok(BrowserLoginStart {
            request_id,
            request_secret,
            authorization_url,
            expires_at_unix,
        })
    }

    pub(crate) async fn complete_browser_login(
        &self,
        request_id: &str,
        user: &UserAccount,
        now_unix: u64,
    ) -> Result<String, ApiError> {
        let callback_code = random_token(
            CLI_CALLBACK_CODE_PREFIX,
            "failed to generate CLI callback code",
        )?;
        let completion = self
            .store
            .complete_cli_browser_login(request_id, token_hash(&callback_code), user, now_unix)
            .await?;
        browser_callback_url(
            &completion.callback_url,
            &completion.request_id,
            &callback_code,
        )
    }

    pub(crate) async fn exchange_browser_login(
        &self,
        request_id: &str,
        request_secret: &str,
        callback_code: &str,
        now_unix: u64,
    ) -> Result<CliSessionToken, ApiError> {
        let (session_token, session) = new_cli_session(now_unix)?;
        let expires_at_unix = session.expires_at_unix;
        let identity = self
            .store
            .exchange_cli_browser_login(
                request_id,
                &token_hash(request_secret),
                &token_hash(callback_code),
                session,
                now_unix,
            )
            .await?;
        Ok(CliSessionToken {
            session_token,
            expires_at_unix,
            identity,
        })
    }

    pub(crate) async fn create_exchange_grant(
        &self,
        user: &UserAccount,
        now_unix: u64,
    ) -> Result<CliExchangeGrant, ApiError> {
        let exchange_token = random_token(
            CLI_EXCHANGE_GRANT_PREFIX,
            "failed to generate CLI exchange token",
        )?;
        let expires_at_unix = now_unix + CLI_EXCHANGE_GRANT_TTL_SECS;
        self.store
            .create_cli_exchange_grant(
                CreateCliExchangeGrantCommand {
                    grant_hash: token_hash(&exchange_token),
                    created_at_unix: now_unix,
                    expires_at_unix,
                },
                user,
                now_unix,
            )
            .await?;
        Ok(CliExchangeGrant {
            exchange_token,
            expires_at_unix,
        })
    }

    pub(crate) async fn exchange_grant(
        &self,
        exchange_token: &str,
        now_unix: u64,
    ) -> Result<CliSessionToken, ApiError> {
        let (session_token, session) = new_cli_session(now_unix)?;
        let expires_at_unix = session.expires_at_unix;
        let identity = self
            .store
            .exchange_cli_grant_by_hash(&token_hash(exchange_token), session, now_unix)
            .await?;
        Ok(CliSessionToken {
            session_token,
            expires_at_unix,
            identity,
        })
    }
}

fn new_cli_session(now_unix: u64) -> Result<(String, NewCliSession), ApiError> {
    let id = random_token(CLI_SESSION_ID_PREFIX, "failed to generate CLI session ID")?;
    let session_token = random_token(
        CLI_SESSION_TOKEN_PREFIX,
        "failed to generate CLI session token",
    )?;
    Ok((
        session_token.clone(),
        NewCliSession {
            id,
            token_hash: token_hash(&session_token),
            label: "Scope CLI".to_string(),
            created_at_unix: now_unix,
            expires_at_unix: now_unix + CLI_SESSION_TTL_SECS,
        },
    ))
}

fn random_user_code() -> Result<String, ApiError> {
    let mut bytes = [0_u8; USER_CODE_BYTES];
    getrandom::fill(&mut bytes).map_err(|error| {
        ApiError::internal_message(format!("failed to generate CLI login code: {error}"))
    })?;
    Ok(hex::encode_upper(bytes))
}

fn normalize_user_code(value: &str) -> String {
    value
        .trim()
        .chars()
        .filter(|ch| !ch.is_whitespace() && *ch != '-')
        .flat_map(char::to_uppercase)
        .collect()
}

fn browser_authorization_url(app_origin: &str, request_id: &str) -> Result<String, ApiError> {
    let mut url = Url::parse(app_origin)
        .map_err(|error| ApiError::internal_message(format!("invalid app origin: {error}")))?;
    url.set_path("/cli-login");
    url.set_query(None);
    url.query_pairs_mut().append_pair("request_id", request_id);
    Ok(url.to_string())
}

fn browser_callback_url(
    callback_url: &str,
    request_id: &str,
    callback_code: &str,
) -> Result<String, ApiError> {
    let mut url = validate_loopback_callback_url(callback_url)?;
    url.query_pairs_mut()
        .append_pair("request_id", request_id)
        .append_pair("code", callback_code);
    Ok(url.to_string())
}

fn validate_loopback_callback_url(callback_url: &str) -> Result<Url, ApiError> {
    let url = Url::parse(callback_url)
        .map_err(|_| ApiError::bad_request("CLI callback URL must be a valid URL"))?;
    if url.scheme() != "http" {
        return Err(ApiError::bad_request("CLI callback URL must use http"));
    }
    if url.port().is_none() {
        return Err(ApiError::bad_request(
            "CLI callback URL must include a port",
        ));
    }
    if url.path() != "/scope-cli-callback" {
        return Err(ApiError::bad_request(
            "CLI callback URL must use /scope-cli-callback",
        ));
    }
    if url.query().is_some() {
        return Err(ApiError::bad_request(
            "CLI callback URL must not include query parameters",
        ));
    }
    if url.fragment().is_some() {
        return Err(ApiError::bad_request(
            "CLI callback URL must not include a fragment",
        ));
    }
    let Some(host) = url.host_str() else {
        return Err(ApiError::bad_request(
            "CLI callback URL must include a host",
        ));
    };
    if !matches!(host, "127.0.0.1" | "localhost" | "::1") {
        return Err(ApiError::bad_request(
            "CLI callback URL must use localhost or 127.0.0.1",
        ));
    }
    Ok(url)
}
