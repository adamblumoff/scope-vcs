use anyhow::Context;
use reqwest::{StatusCode, blocking::Client};
use scope_core::domain::repo_config::RepoConfig;
use scope_core::domain::requests::{
    RequestActorRole, RequestBaseAudience, RequestDisposition, RequestEventKind, RequestState,
};
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
    pub open_request_count: usize,
    pub request_permissions: RepoRequestPermissionsResponse,
}

#[derive(Deserialize)]
pub struct RepositoryAccessResponse {
    pub actor: RepositoryActor,
    pub can_push: bool,
}

#[derive(Deserialize)]
pub struct RepoRequestPermissionsResponse {
    pub can_submit_request: bool,
    pub uses_credit_stake: bool,
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
pub struct RepoConfigResponse {
    pub config: RepoConfig,
    pub config_hash: String,
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

pub struct FinalizeRequestSubmissionParams<'a> {
    pub owner: &'a str,
    pub repo: &'a str,
    pub request_id: &'a str,
    pub title: String,
    pub head_oid: String,
    pub stake_credits: Option<u32>,
}

pub struct ResolveRequestParams<'a> {
    pub owner: &'a str,
    pub repo: &'a str,
    pub request_id: &'a str,
    pub disposition: RequestDisposition,
    pub body: Option<String>,
}

pub struct MergeRequestParams<'a> {
    pub owner: &'a str,
    pub repo: &'a str,
    pub request_id: &'a str,
    pub expected_main_oid: String,
    pub expected_head_oid: String,
    pub body: Option<String>,
}

#[derive(Deserialize)]
pub struct RequestListResponse {
    pub requests: Vec<RequestSummaryResponse>,
}

#[derive(Deserialize)]
pub struct RequestDetailResponse {
    pub request: RequestSummaryResponse,
    pub events: Vec<RequestEventResponse>,
}

#[derive(Deserialize)]
pub struct RequestMutationResponse {
    pub request: RequestSummaryResponse,
}

#[derive(Deserialize)]
pub struct RequestReservationResponse {
    pub id: String,
    pub request_ref: String,
    pub base_audience: RequestBaseAudience,
    pub base_main_oid: String,
}

#[derive(Clone, Debug, Deserialize)]
pub struct RequestSummaryResponse {
    pub id: String,
    pub title: String,
    pub author_user_id: String,
    pub author_role: RequestActorRole,
    pub base_audience: RequestBaseAudience,
    pub target_branch: String,
    pub request_ref: String,
    pub base_main_oid: String,
    pub head_oid: String,
    pub state: RequestState,
    pub stake_credits: u32,
    pub disposition: Option<RequestDisposition>,
    pub settlement: Option<RequestSettlementResponse>,
    pub created_at_unix: u64,
    pub updated_at_unix: u64,
    pub resolved_at_unix: Option<u64>,
    pub permissions: RequestPermissionsResponse,
    pub mergeability: RequestMergeabilityResponse,
}

#[derive(Clone, Debug, Deserialize)]
pub struct RequestPermissionsResponse {
    pub can_comment: bool,
    pub can_update_branch: bool,
    pub can_mark_needs_response: bool,
    pub can_respond: bool,
    pub can_resolve: bool,
    pub can_merge: bool,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq)]
pub enum RequestMergeabilityStatus {
    Ready,
    Closed,
    NotMaintainer,
    MissingRequestBranch,
}

#[derive(Clone, Debug, Deserialize)]
pub struct RequestMergeabilityResponse {
    pub status: RequestMergeabilityStatus,
    pub current_main_oid: Option<String>,
    pub request_head_oid: String,
    pub reason: Option<String>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct RequestSettlementResponse {
    pub disposition: RequestDisposition,
    pub stake_credits: u32,
    pub refunded_credits: u32,
    pub reward_credits: u32,
    pub burned_credits: u32,
    pub settled_at_unix: u64,
}

#[derive(Clone, Debug, Deserialize)]
pub struct RequestEventResponse {
    pub id: String,
    pub actor_user_id: String,
    pub kind: RequestEventKind,
    pub body: Option<String>,
    pub old_head_oid: Option<String>,
    pub new_head_oid: Option<String>,
    pub created_at_unix: u64,
}

#[derive(Serialize)]
struct FinalizeRequestSubmissionRequest {
    title: String,
    head_oid: String,
    stake_credits: Option<u32>,
}

#[derive(Serialize)]
struct CommentRequestRequest {
    body: String,
}

#[derive(Serialize)]
struct NeedsResponseRequest {
    body: String,
}

#[derive(Serialize)]
struct RespondRequestRequest {
    body: Option<String>,
}

#[derive(Serialize)]
struct ResolveRequestRequest {
    disposition: RequestDisposition,
    body: Option<String>,
}

#[derive(Serialize)]
struct MergeRequestRequest {
    expected_main_oid: String,
    expected_head_oid: String,
    body: Option<String>,
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
    let request = CreateRepoRequest { name };
    let response = client
        .post(format!("{api_url}/v1/repos"))
        .bearer_auth(session_token)
        .json(&request)
        .send()
        .context("create Scope repository")?;
    if response.status() == StatusCode::CONFLICT {
        anyhow::bail!("{}", duplicate_repo_error_message(&request.name));
    }

    response
        .error_for_status()
        .context("create Scope repository")?
        .json()
        .context("parse create repository response")
}

fn duplicate_repo_error_message(name: &str) -> String {
    format!(
        "Scope repository {name:?} already exists for this account. Use `scope init --name <new-name>` to create a different repo, or run `scope push` if this checkout is already linked to Scope."
    )
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

pub fn list_requests(
    client: &Client,
    api_url: &str,
    session_token: &str,
    owner: &str,
    repo: &str,
) -> anyhow::Result<RequestListResponse> {
    let response = client
        .get(format!("{api_url}/v1/repos/{owner}/{repo}/requests"))
        .bearer_auth(session_token)
        .send()
        .with_context(|| format!("list requests for {owner}/{repo}"))?;
    handle_repo_request_status(response.status(), owner, repo, "list requests")?;
    response
        .error_for_status()
        .with_context(|| format!("list requests for {owner}/{repo}"))?
        .json()
        .context("parse request list response")
}

pub fn get_request(
    client: &Client,
    api_url: &str,
    session_token: &str,
    owner: &str,
    repo: &str,
    request_id: &str,
) -> anyhow::Result<RequestDetailResponse> {
    let response = client
        .get(format!(
            "{api_url}/v1/repos/{owner}/{repo}/requests/{request_id}"
        ))
        .bearer_auth(session_token)
        .send()
        .with_context(|| format!("load request {request_id} for {owner}/{repo}"))?;
    handle_request_status(response.status(), owner, repo, request_id, "load request")?;
    response
        .error_for_status()
        .with_context(|| format!("load request {request_id} for {owner}/{repo}"))?
        .json()
        .context("parse request detail response")
}

pub fn reserve_request(
    client: &Client,
    api_url: &str,
    session_token: &str,
    owner: &str,
    repo: &str,
) -> anyhow::Result<RequestReservationResponse> {
    let response = client
        .post(format!(
            "{api_url}/v1/repos/{owner}/{repo}/requests/reservations"
        ))
        .bearer_auth(session_token)
        .send()
        .with_context(|| format!("reserve request upload for {owner}/{repo}"))?;
    handle_repo_request_status(response.status(), owner, repo, "reserve request upload")?;
    response
        .error_for_status()
        .with_context(|| format!("reserve request upload for {owner}/{repo}"))?
        .json()
        .context("parse request reservation response")
}

pub fn finalize_request_submission(
    client: &Client,
    api_url: &str,
    session_token: &str,
    params: FinalizeRequestSubmissionParams<'_>,
) -> anyhow::Result<RequestMutationResponse> {
    let response = client
        .post(format!(
            "{api_url}/v1/repos/{}/{}/requests/{}/submit",
            params.owner, params.repo, params.request_id
        ))
        .bearer_auth(session_token)
        .json(&FinalizeRequestSubmissionRequest {
            title: params.title,
            head_oid: params.head_oid,
            stake_credits: params.stake_credits,
        })
        .send()
        .with_context(|| {
            format!(
                "finalize request {} for {}/{}",
                params.request_id, params.owner, params.repo
            )
        })?;
    handle_repo_request_status(
        response.status(),
        params.owner,
        params.repo,
        "finalize request submission",
    )?;
    response
        .error_for_status()
        .with_context(|| {
            format!(
                "finalize request {} for {}/{}",
                params.request_id, params.owner, params.repo
            )
        })?
        .json()
        .context("parse finalize request response")
}

pub fn comment_request(
    client: &Client,
    api_url: &str,
    session_token: &str,
    owner: &str,
    repo: &str,
    request_id: &str,
    body: String,
) -> anyhow::Result<RequestMutationResponse> {
    request_mutation(
        client,
        api_url,
        session_token,
        RequestMutationEndpoint {
            owner,
            repo,
            request_id,
            action_path: "comments",
            context: "comment request",
        },
        &CommentRequestRequest { body },
    )
}

pub fn mark_request_needs_response(
    client: &Client,
    api_url: &str,
    session_token: &str,
    owner: &str,
    repo: &str,
    request_id: &str,
    body: String,
) -> anyhow::Result<RequestMutationResponse> {
    request_mutation(
        client,
        api_url,
        session_token,
        RequestMutationEndpoint {
            owner,
            repo,
            request_id,
            action_path: "needs-response",
            context: "mark request needs response",
        },
        &NeedsResponseRequest { body },
    )
}

pub fn respond_to_request(
    client: &Client,
    api_url: &str,
    session_token: &str,
    owner: &str,
    repo: &str,
    request_id: &str,
    body: Option<String>,
) -> anyhow::Result<RequestMutationResponse> {
    request_mutation(
        client,
        api_url,
        session_token,
        RequestMutationEndpoint {
            owner,
            repo,
            request_id,
            action_path: "respond",
            context: "respond to request",
        },
        &RespondRequestRequest { body },
    )
}

pub fn resolve_request(
    client: &Client,
    api_url: &str,
    session_token: &str,
    params: ResolveRequestParams<'_>,
) -> anyhow::Result<RequestMutationResponse> {
    request_mutation(
        client,
        api_url,
        session_token,
        RequestMutationEndpoint {
            owner: params.owner,
            repo: params.repo,
            request_id: params.request_id,
            action_path: "resolve",
            context: "resolve request",
        },
        &ResolveRequestRequest {
            disposition: params.disposition,
            body: params.body,
        },
    )
}

pub fn merge_request(
    client: &Client,
    api_url: &str,
    session_token: &str,
    params: MergeRequestParams<'_>,
) -> anyhow::Result<RequestMutationResponse> {
    request_mutation(
        client,
        api_url,
        session_token,
        RequestMutationEndpoint {
            owner: params.owner,
            repo: params.repo,
            request_id: params.request_id,
            action_path: "merge",
            context: "merge request",
        },
        &MergeRequestRequest {
            expected_main_oid: params.expected_main_oid,
            expected_head_oid: params.expected_head_oid,
            body: params.body,
        },
    )
}

struct RequestMutationEndpoint<'a> {
    owner: &'a str,
    repo: &'a str,
    request_id: &'a str,
    action_path: &'static str,
    context: &'static str,
}

fn request_mutation<T: Serialize>(
    client: &Client,
    api_url: &str,
    session_token: &str,
    endpoint: RequestMutationEndpoint<'_>,
    body: &T,
) -> anyhow::Result<RequestMutationResponse> {
    let response = client
        .post(format!(
            "{api_url}/v1/repos/{}/{}/requests/{}/{}",
            endpoint.owner, endpoint.repo, endpoint.request_id, endpoint.action_path
        ))
        .bearer_auth(session_token)
        .json(body)
        .send()
        .with_context(|| {
            format!(
                "{} {} for {}/{}",
                endpoint.context, endpoint.request_id, endpoint.owner, endpoint.repo
            )
        })?;
    handle_request_status(
        response.status(),
        endpoint.owner,
        endpoint.repo,
        endpoint.request_id,
        endpoint.context,
    )?;
    response
        .error_for_status()
        .with_context(|| {
            format!(
                "{} {} for {}/{}",
                endpoint.context, endpoint.request_id, endpoint.owner, endpoint.repo
            )
        })?
        .json()
        .with_context(|| format!("parse {} response", endpoint.context))
}

fn handle_repo_request_status(
    status: StatusCode,
    owner: &str,
    repo: &str,
    action: &str,
) -> anyhow::Result<()> {
    match status {
        StatusCode::UNAUTHORIZED => anyhow::bail!("not signed in; run scope login"),
        StatusCode::FORBIDDEN => anyhow::bail!("{action} is not allowed for {owner}/{repo}"),
        StatusCode::NOT_FOUND => anyhow::bail!("repo {owner}/{repo} not found"),
        StatusCode::CONFLICT => anyhow::bail!("{action} conflicted for {owner}/{repo}"),
        _ => Ok(()),
    }
}

fn handle_request_status(
    status: StatusCode,
    owner: &str,
    repo: &str,
    request_id: &str,
    action: &str,
) -> anyhow::Result<()> {
    match status {
        StatusCode::UNAUTHORIZED => anyhow::bail!("not signed in; run scope login"),
        StatusCode::FORBIDDEN => anyhow::bail!("{action} is not allowed for request {request_id}"),
        StatusCode::NOT_FOUND => {
            anyhow::bail!("request {request_id} not found in {owner}/{repo}")
        }
        StatusCode::CONFLICT => anyhow::bail!("{action} conflicted for request {request_id}"),
        _ => Ok(()),
    }
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
mod tests;
