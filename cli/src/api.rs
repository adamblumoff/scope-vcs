use crate::error::CliError;
use anyhow::Context;
use reqwest::{
    StatusCode,
    blocking::{Client, ClientBuilder, Response},
    header::{HeaderMap, HeaderValue},
};
pub use scope_api_contract::routes::{
    cli_browser_login_exchange as cli_browser_login_exchange_path,
    cli_device_login_poll as cli_device_login_poll_path,
};
pub use scope_api_contract::*;
use scope_domain::repo_config::RepoConfig as DomainRepoConfig;
use serde::de::DeserializeOwned;
use std::{env, time::Duration};

mod requests;
mod runs;
pub use requests::*;
pub use runs::*;

const DEFAULT_API_URL: &str = "https://scope-api-production-0251.up.railway.app";
pub const ACCOUNT_SESSION_PATH: &str = scope_api_contract::routes::ACCOUNT_SESSION;
pub const CLI_BROWSER_LOGIN_PATH: &str = scope_api_contract::routes::CLI_BROWSER_LOGIN;
pub const CLI_BROWSER_LOGIN_EXCHANGE_PATH_TEMPLATE: &str =
    scope_api_contract::routes::CLI_BROWSER_LOGIN_EXCHANGE;
pub const CLI_DEVICE_LOGIN_PATH: &str = scope_api_contract::routes::CLI_DEVICE_LOGIN;
pub const CLI_DEVICE_LOGIN_POLL_PATH_TEMPLATE: &str =
    scope_api_contract::routes::CLI_DEVICE_LOGIN_POLL;
pub const CLI_EXCHANGE_GRANTS_EXCHANGE_PATH: &str =
    scope_api_contract::routes::CLI_EXCHANGE_GRANTS_EXCHANGE;
pub const CLI_SESSION_PATH: &str = scope_api_contract::routes::CLI_SESSION;

pub struct AuthenticatedSession {
    pub token: String,
    pub user: UserResponse,
}

pub struct CreatePushIntentParams<'a> {
    pub owner: &'a str,
    pub repo: &'a str,
    pub head_oid: &'a str,
    pub base_config_hash: &'a str,
    pub config: &'a DomainRepoConfig,
}

pub struct RepoConfigContext {
    pub config: DomainRepoConfig,
    pub config_hash: String,
    pub lifecycle_state: RepoPublicationState,
    pub access: RepositoryAccessResponse,
    pub head_oid: Option<String>,
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
    http_client_builder()
        .timeout(Duration::from_secs(20))
        .build()
        .context("build HTTP client")
}

pub(crate) fn decode_json_response<T: DeserializeOwned>(
    response: Response,
    context: &str,
) -> anyhow::Result<T> {
    let response = successful_response(response, context)?;
    response
        .json()
        .with_context(|| format!("parse {context} response"))
}

pub(crate) fn successful_response(response: Response, context: &str) -> anyhow::Result<Response> {
    if response.status().is_success() {
        return Ok(response);
    }

    let status = response.status();
    let mut error = response
        .json::<ErrorResponse>()
        .unwrap_or_else(|_| fallback_error_response(status, context));
    error.message = terminal_safe(&error.message);
    error.instruction = error
        .instruction
        .as_deref()
        .map(terminal_safe)
        .filter(|instruction| !instruction.is_empty());
    error.fields.paths = error
        .fields
        .paths
        .iter()
        .map(|path| terminal_safe(path))
        .filter(|path| !path.is_empty())
        .collect();
    Err(CliError::new(error).into())
}

fn fallback_error_response(status: StatusCode, context: &str) -> ErrorResponse {
    let (code, message, retryable) = match status {
        StatusCode::BAD_REQUEST => (
            ErrorCode::BadRequest,
            format!("Scope rejected the request while trying to {context}"),
            false,
        ),
        StatusCode::UNAUTHORIZED => (
            ErrorCode::Unauthorized,
            "not signed in; run scope login".to_string(),
            false,
        ),
        StatusCode::FORBIDDEN => (
            ErrorCode::Forbidden,
            format!("permission denied while trying to {context}"),
            false,
        ),
        StatusCode::NOT_FOUND => (
            ErrorCode::NotFound,
            format!("the requested Scope resource was not found while trying to {context}"),
            false,
        ),
        StatusCode::CONFLICT => (
            ErrorCode::Conflict,
            format!("Scope state changed while trying to {context}; reload and retry"),
            false,
        ),
        StatusCode::TOO_MANY_REQUESTS => (
            ErrorCode::TooManyRequests,
            format!("Scope is temporarily rate limiting {context}"),
            true,
        ),
        StatusCode::SERVICE_UNAVAILABLE | StatusCode::BAD_GATEWAY | StatusCode::GATEWAY_TIMEOUT => {
            (
                ErrorCode::ServiceUnavailable,
                format!("Scope is temporarily unavailable while trying to {context}"),
                true,
            )
        }
        StatusCode::UPGRADE_REQUIRED => (
            ErrorCode::CliUpgradeRequired,
            format!("{context} requires a newer Scope CLI"),
            false,
        ),
        _ => (
            ErrorCode::Internal,
            format!("Scope could not {context}; retry or contact support if this persists"),
            false,
        ),
    };
    let mut response = ErrorResponse::new(code, message);
    response.retryable = retryable;
    if code == ErrorCode::CliUpgradeRequired {
        response.instruction = Some(format!("Upgrade with `{CLI_INSTALL_COMMAND}`, then retry."));
    }
    response
}

fn terminal_safe(value: &str) -> String {
    value
        .trim()
        .chars()
        .map(|character| {
            if character.is_control() {
                ' '
            } else {
                character
            }
        })
        .collect()
}

pub(crate) fn http_client_builder() -> ClientBuilder {
    Client::builder().default_headers(cli_identity_headers())
}

fn cli_identity_headers() -> HeaderMap {
    let mut headers = HeaderMap::new();
    headers.insert(
        CLI_PROTOCOL_HEADER,
        HeaderValue::from_str(&CLI_PROTOCOL_VERSION.to_string())
            .expect("CLI protocol is a valid header value"),
    );
    headers.insert(
        CLI_VERSION_HEADER,
        HeaderValue::from_static(crate::build::PACKAGE_VERSION),
    );
    headers.insert(
        CLI_BUILD_HEADER,
        HeaderValue::from_static(crate::build::BUILD_SHA),
    );
    headers
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

    let session: AccountSessionResponse =
        decode_json_response(response, "validate saved Scope login")?;
    let AccountSessionResponse { identity, user, .. } = session;
    drop(identity);
    Ok(user)
}

pub fn account_session(
    client: &Client,
    api_url: &str,
    session_token: &str,
) -> anyhow::Result<AccountSessionResponse> {
    let response = client
        .get(format!("{api_url}{ACCOUNT_SESSION_PATH}"))
        .bearer_auth(session_token)
        .send()
        .context("load Scope account")?;
    decode_json_response(response, "load Scope account")
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

    successful_response(response, "revoke Scope CLI session")?;
    Ok(())
}

pub fn create_repo(
    client: &Client,
    api_url: &str,
    session_token: &str,
    name: String,
) -> anyhow::Result<CreateRepoResponse> {
    let request = CreateRepoRequest {
        name,
        visibility: None,
    };
    let response = client
        .post(format!("{api_url}{}", scope_api_contract::routes::REPOS))
        .bearer_auth(session_token)
        .json(&request)
        .send()
        .context("create Scope repository")?;
    decode_json_response(response, "create Scope repository")
}

pub fn get_repo(
    client: &Client,
    api_url: &str,
    session_token: &str,
    owner: &str,
    repo: &str,
) -> anyhow::Result<RepoSummaryResponse> {
    let response = client
        .get(format!(
            "{api_url}{}",
            scope_api_contract::routes::repo(owner, repo)
        ))
        .bearer_auth(session_token)
        .send()
        .with_context(|| format!("load Scope repo {owner}/{repo}"))?;
    decode_json_response(response, &format!("load Scope repo {owner}/{repo}"))
}

pub fn get_repo_config(
    client: &Client,
    api_url: &str,
    session_token: &str,
    owner: &str,
    repo: &str,
) -> anyhow::Result<RepoConfigContext> {
    let response = client
        .get(format!(
            "{api_url}{}",
            scope_api_contract::routes::repo_config(owner, repo)
        ))
        .bearer_auth(session_token)
        .send()
        .with_context(|| format!("get repo config for {owner}/{repo}"))?;
    let response: RepoConfigResponse =
        decode_json_response(response, &format!("get repo config for {owner}/{repo}"))?;
    Ok(RepoConfigContext {
        config: response.config.into(),
        config_hash: response.config_hash,
        lifecycle_state: response.lifecycle_state,
        access: response.access,
        head_oid: response.head_oid,
    })
}

pub fn create_push_intent(
    client: &Client,
    api_url: &str,
    session_token: &str,
    params: CreatePushIntentParams<'_>,
) -> anyhow::Result<CreatePushIntentResponse> {
    let response = client
        .post(format!(
            "{api_url}{}",
            scope_api_contract::routes::repo_push_intents(params.owner, params.repo)
        ))
        .bearer_auth(session_token)
        .json(&CreatePushIntentRequest {
            head_oid: params.head_oid.to_string(),
            base_config_hash: params.base_config_hash.to_string(),
            config: params.config.clone().into(),
        })
        .send()
        .with_context(|| format!("create push intent for {}/{}", params.owner, params.repo))?;
    decode_json_response(
        response,
        &format!("create push intent for {}/{}", params.owner, params.repo),
    )
}

pub fn rollback_created_repo(
    client: &Client,
    api_url: &str,
    session_token: &str,
    repo: &RepoSummaryResponse,
) {
    let result = client
        .delete(format!(
            "{api_url}{}",
            scope_api_contract::routes::repo(&repo.owner_handle, &repo.name)
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
    use std::{
        io::{Read, Write},
        net::TcpListener,
        thread,
    };

    #[test]
    fn shared_http_client_sends_cli_compatibility_identity() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let server = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut bytes = [0_u8; 8192];
            let read = stream.read(&mut bytes).unwrap();
            stream
                .write_all(b"HTTP/1.1 204 No Content\r\nConnection: close\r\n\r\n")
                .unwrap();
            String::from_utf8(bytes[..read].to_vec()).unwrap()
        });

        let response = http_client()
            .unwrap()
            .get(format!("http://{address}/identity"))
            .send()
            .unwrap();
        assert_eq!(response.status(), StatusCode::NO_CONTENT);

        let request = server.join().unwrap().to_ascii_lowercase();
        assert!(request.contains("x-scope-cli-protocol: 1\r\n"));
        assert!(request.contains(&format!(
            "x-scope-cli-version: {}\r\n",
            crate::build::PACKAGE_VERSION
        )));
        assert!(request.contains(&format!(
            "x-scope-cli-build: {}\r\n",
            crate::build::BUILD_SHA
        )));
    }
}
