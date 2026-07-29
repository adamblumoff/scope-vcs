use super::*;
use anyhow::Context;
use reqwest::{
    StatusCode,
    blocking::{Client, Response},
};
use scope_api_contract::{
    AppendAttemptLogRequest, AttachRunnerRepositoryRequest, AttemptHeartbeatRequest,
    AttemptStatusResponse, ClaimRunResponse, CompleteAttemptRequest, CreateManualRunQuery,
    RegisterRunnerRequest, RegisterRunnerResponse, RunEventsQuery, RunEventsResponse,
    RunLogResponse, RunResponse, RunnerPollResponse, RunnerResponse,
};

pub fn register_runner(
    client: &Client,
    api_url: &str,
    session_token: &str,
    request: &RegisterRunnerRequest,
) -> anyhow::Result<RegisterRunnerResponse> {
    parse_json(
        client
            .post(format!("{api_url}{}", routes::RUNNERS))
            .bearer_auth(session_token)
            .json(request)
            .send()
            .context("register Scope runner")?,
        "register Scope runner",
    )
}

pub fn get_runner(
    client: &Client,
    api_url: &str,
    session_token: &str,
    runner_id: &str,
) -> anyhow::Result<RunnerResponse> {
    parse_json(
        client
            .get(format!("{api_url}{}", routes::runner(runner_id)))
            .bearer_auth(session_token)
            .send()
            .context("load Scope runner")?,
        "load Scope runner",
    )
}

pub fn delete_runner(
    client: &Client,
    api_url: &str,
    session_token: &str,
    runner_id: &str,
) -> anyhow::Result<()> {
    successful(
        client
            .delete(format!("{api_url}{}", routes::runner(runner_id)))
            .bearer_auth(session_token)
            .send()
            .context("roll back Scope runner registration")?,
        "roll back Scope runner registration",
    )?;
    Ok(())
}

pub fn attach_runner_repository(
    client: &Client,
    api_url: &str,
    session_token: &str,
    runner_id: &str,
    owner: &str,
    repo: &str,
    name: &str,
) -> anyhow::Result<RunnerResponse> {
    parse_json(
        client
            .put(format!(
                "{api_url}{}",
                routes::runner_repository(runner_id, owner, repo)
            ))
            .bearer_auth(session_token)
            .json(&AttachRunnerRepositoryRequest {
                name: name.to_string(),
            })
            .send()
            .context("attach Scope runner repository")?,
        "attach Scope runner repository",
    )
}

pub fn detach_runner_repository(
    client: &Client,
    api_url: &str,
    session_token: &str,
    runner_id: &str,
    owner: &str,
    repo: &str,
) -> anyhow::Result<RunnerResponse> {
    parse_json(
        client
            .delete(format!(
                "{api_url}{}",
                routes::runner_repository(runner_id, owner, repo)
            ))
            .bearer_auth(session_token)
            .send()
            .context("detach Scope runner repository")?,
        "detach Scope runner repository",
    )
}

#[allow(clippy::too_many_arguments)]
pub fn create_manual_run(
    client: &Client,
    api_url: &str,
    session_token: &str,
    owner: &str,
    repo: &str,
    query: &CreateManualRunQuery,
    bundle: Vec<u8>,
) -> anyhow::Result<RunResponse> {
    parse_json(
        client
            .post(format!("{api_url}{}", routes::repo_runs(owner, repo)))
            .bearer_auth(session_token)
            .query(query)
            .header("content-type", "application/octet-stream")
            .body(bundle)
            .send()
            .context("create Scope run")?,
        "create Scope run",
    )
}

pub fn get_run_events(
    client: &Client,
    api_url: &str,
    session_token: &str,
    owner: &str,
    repo: &str,
    run_id: &str,
    after: u64,
) -> anyhow::Result<RunEventsResponse> {
    parse_json(
        client
            .get(format!(
                "{api_url}{}",
                routes::repo_run_events(owner, repo, run_id)
            ))
            .bearer_auth(session_token)
            .query(&RunEventsQuery { after })
            .send()
            .context("watch Scope run")?,
        "watch Scope run",
    )
}

pub fn cancel_run(
    client: &Client,
    api_url: &str,
    session_token: &str,
    owner: &str,
    repo: &str,
    run_id: &str,
) -> anyhow::Result<RunResponse> {
    mutate_run(
        client,
        api_url,
        session_token,
        routes::repo_run_cancel(owner, repo, run_id),
        "cancel Scope run",
    )
}

pub fn retry_run(
    client: &Client,
    api_url: &str,
    session_token: &str,
    owner: &str,
    repo: &str,
    run_id: &str,
) -> anyhow::Result<RunResponse> {
    mutate_run(
        client,
        api_url,
        session_token,
        routes::repo_run_retry(owner, repo, run_id),
        "retry Scope run",
    )
}

fn mutate_run(
    client: &Client,
    api_url: &str,
    session_token: &str,
    path: String,
    context: &str,
) -> anyhow::Result<RunResponse> {
    parse_json(
        client
            .post(format!("{api_url}{path}"))
            .bearer_auth(session_token)
            .send()
            .with_context(|| context.to_string())?,
        context,
    )
}

pub fn runner_poll(
    client: &Client,
    api_url: &str,
    runner_secret: &str,
) -> anyhow::Result<RunnerPollResponse> {
    parse_json(
        client
            .post(format!("{api_url}{}", routes::RUNNER_POLL))
            .bearer_auth(runner_secret)
            .send()
            .context("poll Scope runner queue")?,
        "poll Scope runner queue",
    )
}

pub fn runner_claim(
    client: &Client,
    api_url: &str,
    runner_secret: &str,
    run_id: &str,
) -> anyhow::Result<ClaimRunResponse> {
    parse_json(
        client
            .post(format!("{api_url}{}", routes::runner_claim(run_id)))
            .bearer_auth(runner_secret)
            .send()
            .context("claim Scope run")?,
        "claim Scope run",
    )
}

pub fn attempt_source(
    client: &Client,
    api_url: &str,
    attempt_token: &str,
    attempt_id: &str,
) -> anyhow::Result<Response> {
    successful(
        client
            .get(format!("{api_url}{}", routes::attempt_source(attempt_id)))
            .bearer_auth(attempt_token)
            .send()
            .context("download Scope run source")?,
        "download Scope run source",
    )
}

pub fn attempt_start(
    client: &Client,
    api_url: &str,
    attempt_token: &str,
    attempt_id: &str,
) -> anyhow::Result<AttemptStatusResponse> {
    attempt_json(
        client,
        api_url,
        attempt_token,
        routes::attempt_start(attempt_id),
        &serde_json::json!({}),
        "start Scope run attempt",
    )
}

pub fn attempt_heartbeat(
    client: &Client,
    api_url: &str,
    attempt_token: &str,
    attempt_id: &str,
) -> anyhow::Result<AttemptStatusResponse> {
    attempt_json(
        client,
        api_url,
        attempt_token,
        routes::attempt_heartbeat(attempt_id),
        &AttemptHeartbeatRequest {},
        "heartbeat Scope run attempt",
    )
}

pub fn append_attempt_log(
    client: &Client,
    api_url: &str,
    attempt_token: &str,
    attempt_id: &str,
    request: &AppendAttemptLogRequest,
) -> anyhow::Result<bool> {
    let response = client
        .post(format!("{api_url}{}", routes::attempt_logs(attempt_id)))
        .bearer_auth(attempt_token)
        .json(request)
        .send()
        .context("append Scope run log")?;
    if response.status() == StatusCode::TOO_MANY_REQUESTS {
        return Ok(false);
    }
    let _: RunLogResponse = parse_json(response, "append Scope run log")?;
    Ok(true)
}

pub fn complete_attempt(
    client: &Client,
    api_url: &str,
    attempt_token: &str,
    attempt_id: &str,
    request: &CompleteAttemptRequest,
) -> anyhow::Result<AttemptStatusResponse> {
    attempt_json(
        client,
        api_url,
        attempt_token,
        routes::attempt_complete(attempt_id),
        request,
        "complete Scope run attempt",
    )
}

fn attempt_json<T: serde::Serialize>(
    client: &Client,
    api_url: &str,
    attempt_token: &str,
    path: String,
    body: &T,
    context: &str,
) -> anyhow::Result<AttemptStatusResponse> {
    parse_json(
        client
            .post(format!("{api_url}{path}"))
            .bearer_auth(attempt_token)
            .json(body)
            .send()
            .with_context(|| context.to_string())?,
        context,
    )
}

fn parse_json<T: serde::de::DeserializeOwned>(
    response: Response,
    context: &str,
) -> anyhow::Result<T> {
    let response = successful(response, context)?;
    response
        .json()
        .with_context(|| format!("parse {context} response"))
}

fn successful(response: Response, context: &str) -> anyhow::Result<Response> {
    if response.status().is_success() {
        return Ok(response);
    }
    let status = response.status();
    let message = response
        .json::<serde_json::Value>()
        .ok()
        .and_then(|value| {
            value
                .get("error")
                .and_then(|error| error.as_str())
                .map(str::to_string)
        })
        .unwrap_or_else(|| status.to_string());
    if status == StatusCode::UNAUTHORIZED {
        anyhow::bail!("{context}: authentication failed: {message}");
    }
    anyhow::bail!("{context}: {message} ({status})")
}
