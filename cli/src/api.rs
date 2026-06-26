use anyhow::Context;
use reqwest::{StatusCode, blocking::Client};
use serde::{Deserialize, Serialize};
use std::{env, time::Duration};

const DEFAULT_API_URL: &str = "https://scope-api-production-0251.up.railway.app";

#[derive(Clone, Copy, Debug, Serialize)]
pub enum Visibility {
    Private,
    Public,
}

#[derive(Deserialize)]
pub struct DeviceLoginStartResponse {
    pub device_code: String,
    pub user_code: String,
    pub verification_url: String,
    pub expires_at_unix: u64,
    pub poll_interval_secs: u64,
}

#[derive(Deserialize)]
pub struct DeviceLoginPollResponse {
    pub status: DeviceLoginStatus,
    pub session_token: Option<String>,
}

#[derive(Serialize)]
pub struct BrowserLoginStartRequest {
    pub callback_url: String,
}

#[derive(Deserialize)]
pub struct BrowserLoginStartResponse {
    pub request_id: String,
    pub request_secret: String,
    pub authorization_url: String,
    pub expires_at_unix: u64,
}

#[derive(Serialize)]
pub struct BrowserLoginExchangeRequest {
    pub request_secret: String,
    pub callback_code: String,
}

#[derive(Serialize)]
pub struct CliExchangeGrantExchangeRequest {
    pub exchange_token: String,
}

#[derive(Deserialize)]
pub struct CliSessionTokenResponse {
    pub session_token: String,
}

#[derive(Deserialize)]
pub enum DeviceLoginStatus {
    Pending,
    Complete,
}

pub struct AuthenticatedSession {
    pub token: String,
    pub user: UserResponse,
}

#[derive(Deserialize)]
struct AccountSessionResponse {
    user: Option<UserResponse>,
}

#[derive(Deserialize)]
pub struct UserResponse {
    pub handle: String,
    pub email: String,
}

#[derive(Serialize)]
struct CreateRepoRequest {
    name: String,
    visibility: Visibility,
}

#[derive(Deserialize)]
pub struct CreateRepoResponse {
    pub repo: RepoSummaryResponse,
    pub init: RepoInitResponse,
}

#[derive(Deserialize)]
pub struct RepoSummaryResponse {
    pub owner_handle: String,
    pub name: String,
}

#[derive(Deserialize)]
pub struct RepoInitResponse {
    pub git_remote_url: String,
    pub remote_name: String,
    pub push_branch: String,
    pub push_token: Option<GitPushTokenResponse>,
    pub review_url: String,
}

#[derive(Deserialize)]
pub struct GitPushTokenResponse {
    pub secret: Option<String>,
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
        .get(format!("{api_url}/v1/session"))
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
    Ok(session.user)
}

pub fn revoke_cli_session(
    client: &Client,
    api_url: &str,
    session_token: &str,
) -> anyhow::Result<()> {
    let response = client
        .delete(format!("{api_url}/v1/cli/session"))
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
    visibility: Visibility,
) -> anyhow::Result<CreateRepoResponse> {
    client
        .post(format!("{api_url}/v1/repos"))
        .bearer_auth(session_token)
        .json(&CreateRepoRequest { name, visibility })
        .send()
        .context("create Scope repository")?
        .error_for_status()
        .context("create Scope repository")?
        .json()
        .context("parse create repository response")
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
