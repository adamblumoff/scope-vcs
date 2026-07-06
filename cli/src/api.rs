use anyhow::Context;
use reqwest::{StatusCode, blocking::Client};
use scope_core::domain::repo_config::RepoConfig;
use serde::{Deserialize, Serialize};
use std::{env, time::Duration};

const DEFAULT_API_URL: &str = "https://scope-api-production-0251.up.railway.app";
pub const ACCOUNT_SESSION_PATH: &str = "/v1/session";
pub const CLI_BROWSER_LOGIN_PATH: &str = "/v1/cli/browser-login";
pub const CLI_BROWSER_LOGIN_EXCHANGE_PATH_TEMPLATE: &str =
    "/v1/cli/browser-login/{request_id}/exchange";
pub const CLI_DEVICE_LOGIN_PATH: &str = "/v1/cli/device-login";
pub const CLI_DEVICE_LOGIN_POLL_PATH_TEMPLATE: &str = "/v1/cli/device-login/{device_code}/poll";
pub const CLI_EXCHANGE_GRANTS_EXCHANGE_PATH: &str = "/v1/cli/exchange-grants/exchange";
pub const CLI_SESSION_PATH: &str = "/v1/cli/session";

pub fn cli_browser_login_exchange_path(request_id: &str) -> String {
    format!("/v1/cli/browser-login/{request_id}/exchange")
}

pub fn cli_device_login_poll_path(device_code: &str) -> String {
    format!("/v1/cli/device-login/{device_code}/poll")
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub enum RepositoryActor {
    Public,
    Member,
    Owner,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub enum RepoPublicationState {
    Unpublished,
    Published,
}

#[derive(Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct DeviceLoginStartResponse {
    pub device_code: String,
    pub user_code: String,
    pub verification_url: String,
    pub expires_at_unix: u64,
    pub poll_interval_secs: u64,
}

#[derive(Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct DeviceLoginPollResponse {
    pub status: DeviceLoginStatus,
    pub session_token: Option<String>,
    pub expires_at_unix: u64,
    pub identity: Option<SessionIdentity>,
}

#[derive(Serialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct BrowserLoginStartRequest {
    pub callback_url: String,
}

#[derive(Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct BrowserLoginStartResponse {
    pub request_id: String,
    pub request_secret: String,
    pub authorization_url: String,
    pub expires_at_unix: u64,
}

#[derive(Serialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct BrowserLoginExchangeRequest {
    pub request_secret: String,
    pub callback_code: String,
}

#[derive(Serialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct CliExchangeGrantExchangeRequest {
    pub exchange_token: String,
}

#[derive(Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct CliSessionTokenResponse {
    pub session_token: String,
    pub expires_at_unix: u64,
    pub identity: SessionIdentity,
}

#[derive(Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub enum DeviceLoginStatus {
    Pending,
    Complete,
}

#[derive(Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct SessionIdentity {
    pub user_id: String,
    pub email: Option<String>,
    pub email_verified: bool,
}

pub struct AuthenticatedSession {
    pub token: String,
    pub user: UserResponse,
}

#[derive(Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
struct AccountSessionResponse {
    identity: Option<SessionIdentity>,
    user: Option<UserResponse>,
}

#[derive(Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct UserResponse {
    pub id: String,
    pub handle: String,
    pub email: String,
    pub email_verified: bool,
}

#[derive(Serialize)]
struct CreateRepoRequest {
    name: String,
}

#[derive(Deserialize)]
pub struct CreateRepoResponse {
    pub repo: RepoSummaryResponse,
    pub init: RepoInitResponse,
}

#[derive(Deserialize)]
pub struct RepoSummaryResponse {
    pub id: String,
    pub owner_handle: String,
    pub name: String,
    pub lifecycle_state: RepoPublicationState,
    pub access: RepositoryAccessResponse,
    pub pending_import_pending: bool,
    pub staged_update_pending: bool,
    pub push_blocked_by_staged_update: bool,
}

#[derive(Deserialize)]
pub struct RepositoryAccessResponse {
    pub actor: RepositoryActor,
    pub can_push: bool,
}

#[derive(Deserialize)]
pub struct RepoInitResponse {
    pub git_remote_url: String,
    pub remote_name: String,
    pub push_branch: String,
    pub push_token: Option<GitPushTokenResponse>,
}

#[derive(Deserialize)]
pub struct GitPushTokenResponse {
    pub secret: Option<String>,
}

#[derive(Deserialize)]
pub struct RepoCloneCredentialResponse {
    pub git_remote_path: String,
    pub token: GitCloneTokenResponse,
    pub config: RepoConfig,
}

#[derive(Deserialize)]
pub struct RepoConfigResponse {
    pub config: RepoConfig,
    pub config_hash: String,
}

#[derive(Deserialize)]
pub struct GitCloneTokenResponse {
    pub secret: Option<String>,
}

#[derive(Serialize)]
struct CreatePushIntentRequest {
    head_oid: String,
    base_config_hash: String,
    config: RepoConfig,
}

#[derive(Deserialize)]
pub struct CreatePushIntentResponse {
    pub token: String,
    pub base_head_oid: Option<String>,
    pub expires_at_unix: u64,
}

pub struct CreatePushIntentParams<'a> {
    pub owner: &'a str,
    pub repo: &'a str,
    pub head_oid: &'a str,
    pub base_config_hash: &'a str,
    pub config: &'a RepoConfig,
}

#[derive(Serialize)]
struct CompletePushIntentRequest {
    token: String,
}

pub fn api_url() -> String {
    env::var("SCOPE_API_URL")
        .or_else(|_| env::var("SCOPE_API_PUBLIC_URL"))
        .ok()
        .or_else(|| option_env!("SCOPE_API_URL").map(str::to_string))
        .or_else(|| option_env!("SCOPE_API_PUBLIC_URL").map(str::to_string))
        .unwrap_or_else(|| DEFAULT_API_URL.to_string())
        .trim_end_matches('/')
        .to_string()
}

pub fn http_client() -> anyhow::Result<Client> {
    Client::builder()
        .timeout(Duration::from_secs(20))
        .build()
        .context("build HTTP client")
}

pub fn validate_session_token(
    client: &Client,
    api_url: &str,
    session_token: &str,
) -> anyhow::Result<Option<UserResponse>> {
    let response = client
        .get(format!("{api_url}{ACCOUNT_SESSION_PATH}"))
        .bearer_auth(session_token)
        .send()
        .context("validate saved Scope login")?;
    if response.status() == StatusCode::UNAUTHORIZED {
        return Ok(None);
    }

    let session: AccountSessionResponse = response
        .error_for_status()
        .context("validate saved Scope login")?
        .json()
        .context("parse saved Scope login response")?;
    let AccountSessionResponse { identity, user } = session;
    drop(identity);
    Ok(user)
}

pub fn revoke_cli_session(
    client: &Client,
    api_url: &str,
    session_token: &str,
) -> anyhow::Result<()> {
    let response = client
        .delete(format!("{api_url}{CLI_SESSION_PATH}"))
        .bearer_auth(session_token)
        .send()
        .context("revoke Scope CLI session")?;
    if response.status() == StatusCode::UNAUTHORIZED {
        return Ok(());
    }

    response
        .error_for_status()
        .context("revoke Scope CLI session")?;
    Ok(())
}

pub fn create_repo(
    client: &Client,
    api_url: &str,
    session_token: &str,
    name: String,
) -> anyhow::Result<CreateRepoResponse> {
    client
        .post(format!("{api_url}/v1/repos"))
        .bearer_auth(session_token)
        .json(&CreateRepoRequest { name })
        .send()
        .context("create Scope repository")?
        .error_for_status()
        .context("create Scope repository")?
        .json()
        .context("parse create repository response")
}

pub fn get_repo(
    client: &Client,
    api_url: &str,
    session_token: &str,
    owner: &str,
    repo: &str,
) -> anyhow::Result<RepoSummaryResponse> {
    let response = client
        .get(format!("{api_url}/v1/repos/{owner}/{repo}"))
        .bearer_auth(session_token)
        .send()
        .with_context(|| format!("load Scope repo {owner}/{repo}"))?;
    match response.status() {
        StatusCode::UNAUTHORIZED => {
            anyhow::bail!("not signed in; run scope login")
        }
        StatusCode::NOT_FOUND => {
            anyhow::bail!("repo {owner}/{repo} not found")
        }
        _ => {}
    }

    response
        .error_for_status()
        .with_context(|| format!("load Scope repo {owner}/{repo}"))?
        .json()
        .context("parse repository response")
}

pub fn create_clone_credential(
    client: &Client,
    api_url: &str,
    session_token: &str,
    owner: &str,
    repo: &str,
) -> anyhow::Result<RepoCloneCredentialResponse> {
    let response = client
        .post(format!(
            "{api_url}/v1/repos/{owner}/{repo}/clone-credential"
        ))
        .bearer_auth(session_token)
        .send()
        .with_context(|| format!("create clone credential for {owner}/{repo}"))?;
    match response.status() {
        StatusCode::UNAUTHORIZED => {
            anyhow::bail!("not signed in; run scope login")
        }
        StatusCode::FORBIDDEN => {
            anyhow::bail!("you are not an owner or member of {owner}/{repo}")
        }
        StatusCode::NOT_FOUND => {
            anyhow::bail!("repo {owner}/{repo} not found")
        }
        _ => {}
    }

    response
        .error_for_status()
        .with_context(|| format!("create clone credential for {owner}/{repo}"))?
        .json()
        .context("parse clone credential response")
}

pub fn get_repo_config(
    client: &Client,
    api_url: &str,
    session_token: &str,
    owner: &str,
    repo: &str,
) -> anyhow::Result<RepoConfigResponse> {
    let response = client
        .get(format!("{api_url}/v1/repos/{owner}/{repo}/config"))
        .bearer_auth(session_token)
        .send()
        .with_context(|| format!("get repo config for {owner}/{repo}"))?;
    match response.status() {
        StatusCode::UNAUTHORIZED => {
            anyhow::bail!("not signed in; run scope login")
        }
        StatusCode::FORBIDDEN => {
            anyhow::bail!("repo membership required for {owner}/{repo}")
        }
        StatusCode::NOT_FOUND => {
            anyhow::bail!("repo {owner}/{repo} not found")
        }
        _ => {}
    }

    response
        .error_for_status()
        .with_context(|| format!("get repo config for {owner}/{repo}"))?
        .json()
        .context("parse repo config response")
}

pub fn create_push_intent(
    client: &Client,
    api_url: &str,
    session_token: &str,
    params: CreatePushIntentParams<'_>,
) -> anyhow::Result<CreatePushIntentResponse> {
    let response = client
        .post(format!(
            "{api_url}/v1/repos/{}/{}/push-intents",
            params.owner, params.repo
        ))
        .bearer_auth(session_token)
        .json(&CreatePushIntentRequest {
            head_oid: params.head_oid.to_string(),
            base_config_hash: params.base_config_hash.to_string(),
            config: params.config.clone(),
        })
        .send()
        .with_context(|| format!("create push intent for {}/{}", params.owner, params.repo))?;
    match response.status() {
        StatusCode::UNAUTHORIZED => {
            anyhow::bail!("not signed in; run scope login")
        }
        StatusCode::FORBIDDEN => {
            anyhow::bail!(
                "you do not have write access to {}/{}",
                params.owner,
                params.repo
            )
        }
        StatusCode::NOT_FOUND => {
            anyhow::bail!("repo {}/{} not found", params.owner, params.repo)
        }
        _ => {}
    }

    response
        .error_for_status()
        .with_context(|| format!("create push intent for {}/{}", params.owner, params.repo))?
        .json()
        .context("parse push intent response")
}

pub fn complete_push_intent(
    client: &Client,
    api_url: &str,
    session_token: &str,
    owner: &str,
    repo: &str,
    token: &str,
) -> anyhow::Result<()> {
    let response = client
        .post(format!(
            "{api_url}/v1/repos/{owner}/{repo}/push-intents/complete"
        ))
        .bearer_auth(session_token)
        .json(&CompletePushIntentRequest {
            token: token.to_string(),
        })
        .send()
        .with_context(|| format!("complete push intent for {owner}/{repo}"))?;
    match response.status() {
        StatusCode::UNAUTHORIZED => {
            anyhow::bail!("not signed in; run scope login")
        }
        StatusCode::FORBIDDEN => {
            anyhow::bail!("you do not have write access to {owner}/{repo}")
        }
        StatusCode::NOT_FOUND => {
            anyhow::bail!("repo {owner}/{repo} not found")
        }
        _ => {}
    }

    response
        .error_for_status()
        .with_context(|| format!("complete push intent for {owner}/{repo}"))?;
    Ok(())
}

pub fn rollback_created_repo(
    client: &Client,
    api_url: &str,
    session_token: &str,
    repo: &RepoSummaryResponse,
) {
    let result = client
        .delete(format!(
            "{api_url}/v1/repos/{}/{}",
            repo.owner_handle, repo.name
        ))
        .bearer_auth(session_token)
        .send();

    match result {
        Ok(response) if response.status().is_success() => {
            eprintln!("Deleted Scope repository after failed init");
        }
        Ok(response) => {
            eprintln!(
                "Scope repository was created, but rollback failed: {}",
                response.status()
            );
        }
        Err(error) => {
            eprintln!("Scope repository was created, but rollback failed: {error}");
        }
    }
}

pub fn display_user(user: &UserResponse) -> String {
    if user.email.trim().is_empty() {
        format!("@{}", user.handle)
    } else {
        format!("@{} <{}>", user.handle, user.email)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ts_rs::TS;

    const API_TYPES: &str = include_str!("../../web/src/api/types.generated.ts");

    #[test]
    fn cli_auth_dtos_match_generated_api_contract() {
        assert_type_matches::<SessionIdentity>("SessionIdentity");
        assert_type_matches::<RepositoryActor>("RepositoryActor");
        assert_type_matches::<RepoPublicationState>("RepoPublicationState");
        assert_type_matches::<UserResponse>("UserResponse");
        assert_type_matches::<AccountSessionResponse>("AccountSessionResponse");
        assert_type_matches::<DeviceLoginStatus>("DeviceLoginStatus");
        assert_type_matches::<DeviceLoginStartResponse>("DeviceLoginStartResponse");
        assert_type_matches::<DeviceLoginPollResponse>("DeviceLoginPollResponse");
        assert_type_matches::<BrowserLoginStartRequest>("BrowserLoginStartRequest");
        assert_type_matches::<BrowserLoginStartResponse>("BrowserLoginStartResponse");
        assert_type_matches::<BrowserLoginExchangeRequest>("BrowserLoginExchangeRequest");
        assert_type_matches::<CliSessionTokenResponse>("CliSessionTokenResponse");
        assert_type_matches::<CliExchangeGrantExchangeRequest>("CliExchangeGrantExchangeRequest");
    }

    #[test]
    fn cli_auth_endpoints_match_generated_api_contract() {
        assert_endpoint_matches("accountSession", ACCOUNT_SESSION_PATH);
        assert_endpoint_matches("cliSession", CLI_SESSION_PATH);
        assert_endpoint_matches("deviceLoginStart", CLI_DEVICE_LOGIN_PATH);
        assert_endpoint_matches("deviceLoginPoll", CLI_DEVICE_LOGIN_POLL_PATH_TEMPLATE);
        assert_endpoint_matches("browserLoginStart", CLI_BROWSER_LOGIN_PATH);
        assert_endpoint_matches(
            "browserLoginExchange",
            CLI_BROWSER_LOGIN_EXCHANGE_PATH_TEMPLATE,
        );
        assert_endpoint_matches("exchangeGrantExchange", CLI_EXCHANGE_GRANTS_EXCHANGE_PATH);
    }

    fn assert_type_matches<T: TS>(name: &str) {
        let config = ts_rs::Config::new().with_large_int("number");
        let cli_declaration = format!("export {}", T::decl(&config));
        let api_declaration = exported_type_declaration(name);
        assert_eq!(cli_declaration, api_declaration, "{name} drifted");
    }

    fn exported_type_declaration(name: &str) -> String {
        let prefix = format!("export type {name} = ");
        API_TYPES
            .lines()
            .find(|line| line.starts_with(&prefix))
            .unwrap_or_else(|| panic!("missing generated API declaration for {name}"))
            .to_string()
    }

    fn assert_endpoint_matches(name: &str, cli_path: &str) {
        let api_path = exported_endpoint_path(name);
        assert_eq!(cli_path, api_path, "{name} endpoint drifted");
    }

    fn exported_endpoint_path(name: &str) -> &str {
        let prefix = format!("  {name}: \"");
        let line = API_TYPES
            .lines()
            .find(|line| line.starts_with(&prefix))
            .unwrap_or_else(|| panic!("missing generated API endpoint for {name}"));
        line.strip_prefix(&prefix)
            .and_then(|tail| tail.strip_suffix("\","))
            .unwrap_or_else(|| panic!("invalid generated API endpoint line for {name}"))
    }
}
