use super::*;
use anyhow::Context;
use reqwest::{
    StatusCode,
    blocking::{Client, Response},
};
use scope_api_contract::{
    AppendAttemptLogRequest, AttachRunnerRepositoryRequest, AttemptCacheFinalizationRequest,
    AttemptHeartbeatRequest, AttemptRecoveryStatusResponse, AttemptStatusResponse,
    ClaimRunResponse, CompleteAttemptRequest, CompleteAttemptStepRequest, CreateManualRunQuery,
    PinAttemptContainerImageRequest, PinAttemptContainerImageResponse,
    PushTriggerEvaluationResponse, RegisterRunnerRequest, RegisterRunnerResponse, RunEventsQuery,
    RunLogResponse, RunResponse, RunnerPollResponse, RunnerResponse,
    UpgradeRunnerRegistrationRequest, UpgradeRunnerRegistrationResponse,
};
use std::io::BufRead;

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

pub fn get_push_trigger_evaluation(
    client: &Client,
    api_url: &str,
    session_token: &str,
    owner: &str,
    repo: &str,
    head_oid: &str,
) -> anyhow::Result<PushTriggerEvaluationResponse> {
    parse_json(
        client
            .get(format!(
                "{api_url}{}",
                routes::repo_push_trigger_evaluation(owner, repo, head_oid)
            ))
            .bearer_auth(session_token)
            .send()
            .context("load Scope push trigger evaluation")?,
        "load Scope push trigger evaluation",
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

pub fn upgrade_runner_registration(
    client: &Client,
    api_url: &str,
    session_token: &str,
    runner_id: &str,
    request: &UpgradeRunnerRegistrationRequest,
) -> anyhow::Result<UpgradeRunnerRegistrationResponse> {
    parse_json(
        client
            .post(format!("{api_url}{}", routes::runner_upgrade(runner_id)))
            .bearer_auth(session_token)
            .json(request)
            .send()
            .context("upgrade Scope runner registration")?,
        "upgrade Scope runner registration",
    )
}

pub fn finalize_attempt_cache(
    client: &Client,
    api_url: &str,
    attempt_token: &str,
    attempt_id: &str,
    request: &AttemptCacheFinalizationRequest,
) -> anyhow::Result<()> {
    successful(
        client
            .post(format!(
                "{api_url}{}",
                routes::attempt_cache_finalization(attempt_id)
            ))
            .bearer_auth(attempt_token)
            .json(request)
            .send()
            .context("finalize Scope attempt cache")?,
        "finalize Scope attempt cache",
    )?;
    Ok(())
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

pub enum RunStreamEvent {
    Log(RunLogResponse),
    Status(RunResponse),
}

#[allow(clippy::too_many_arguments)]
pub fn stream_run_events(
    client: &Client,
    api_url: &str,
    session_token: &str,
    owner: &str,
    repo: &str,
    run_id: &str,
    after: u64,
    on_event: impl FnMut(RunStreamEvent) -> anyhow::Result<bool>,
) -> anyhow::Result<()> {
    let response = successful(
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
    )?;
    parse_run_event_stream(std::io::BufReader::new(response), on_event)
}

fn parse_run_event_stream(
    reader: impl BufRead,
    mut on_event: impl FnMut(RunStreamEvent) -> anyhow::Result<bool>,
) -> anyhow::Result<()> {
    let mut event_name = String::new();
    let mut data = Vec::new();
    for line in reader.lines() {
        let line = match line {
            Ok(line) => line,
            Err(_) => return Ok(()),
        };
        if line.is_empty() {
            if !data.is_empty() {
                let payload = data.join("\n");
                let event = match event_name.as_str() {
                    "log" => Some(RunStreamEvent::Log(
                        serde_json::from_str(&payload).context("parse Scope run log event")?,
                    )),
                    "status" => Some(RunStreamEvent::Status(
                        serde_json::from_str(&payload).context("parse Scope run status event")?,
                    )),
                    "error" => {
                        let error: serde_json::Value = serde_json::from_str(&payload)
                            .context("parse Scope run stream error")?;
                        anyhow::bail!(
                            "{}",
                            error["message"]
                                .as_str()
                                .unwrap_or("Scope run event stream failed")
                        );
                    }
                    _ => None,
                };
                if let Some(event) = event
                    && !on_event(event)?
                {
                    return Ok(());
                }
            }
            event_name.clear();
            data.clear();
            continue;
        }
        if line.starts_with(':') {
            continue;
        }
        if let Some(value) = line.strip_prefix("event:") {
            event_name = value.trim_start().to_string();
        } else if let Some(value) = line.strip_prefix("data:") {
            data.push(value.trim_start().to_string());
        }
    }
    Ok(())
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

pub fn start_attempt_step(
    client: &Client,
    api_url: &str,
    attempt_token: &str,
    attempt_id: &str,
    step_index: u32,
) -> anyhow::Result<AttemptStatusResponse> {
    attempt_json(
        client,
        api_url,
        attempt_token,
        routes::attempt_step_start(attempt_id, step_index),
        &serde_json::json!({}),
        "start Scope workflow step",
    )
}

pub fn pin_attempt_container_image(
    client: &Client,
    api_url: &str,
    attempt_token: &str,
    attempt_id: &str,
    image: String,
) -> anyhow::Result<PinAttemptContainerImageResponse> {
    parse_json(
        client
            .post(format!(
                "{api_url}{}",
                routes::attempt_container_image(attempt_id)
            ))
            .bearer_auth(attempt_token)
            .json(&PinAttemptContainerImageRequest { image })
            .send()
            .context("pin Scope run container image")?,
        "pin Scope run container image",
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

pub fn attempt_recovery_status(
    client: &Client,
    api_url: &str,
    attempt_token: &str,
    attempt_id: &str,
) -> anyhow::Result<AttemptRecoveryStatusResponse> {
    parse_json(
        client
            .get(format!(
                "{api_url}{}",
                routes::attempt_recovery_status(attempt_id)
            ))
            .bearer_auth(attempt_token)
            .send()
            .context("load Scope run attempt recovery status")?,
        "load Scope run attempt recovery status",
    )
}

pub enum AttemptRecoveryLookup {
    Active(AttemptRecoveryStatusResponse),
    Unavailable,
}

pub fn attempt_recovery_status_if_active(
    client: &Client,
    api_url: &str,
    attempt_token: &str,
    attempt_id: &str,
) -> anyhow::Result<AttemptRecoveryLookup> {
    let response = client
        .get(format!(
            "{api_url}{}",
            routes::attempt_recovery_status(attempt_id)
        ))
        .bearer_auth(attempt_token)
        .send()
        .context("load Scope run attempt recovery status")?;
    if matches!(
        response.status(),
        StatusCode::UNAUTHORIZED | StatusCode::NOT_FOUND | StatusCode::CONFLICT
    ) {
        return Ok(AttemptRecoveryLookup::Unavailable);
    }
    parse_json(response, "load Scope run attempt recovery status")
        .map(AttemptRecoveryLookup::Active)
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

pub fn complete_attempt_step(
    client: &Client,
    api_url: &str,
    attempt_token: &str,
    attempt_id: &str,
    step_index: u32,
    request: &CompleteAttemptStepRequest,
) -> anyhow::Result<AttemptStatusResponse> {
    attempt_json(
        client,
        api_url,
        attempt_token,
        routes::attempt_step_complete(attempt_id, step_index),
        request,
        "complete Scope workflow step",
    )
}

pub fn abandon_attempt(
    client: &Client,
    api_url: &str,
    attempt_token: &str,
    attempt_id: &str,
) -> anyhow::Result<()> {
    let response = client
        .post(format!("{api_url}{}", routes::attempt_abandon(attempt_id)))
        .bearer_auth(attempt_token)
        .send()
        .context("reconcile interrupted Scope run attempt")?;
    if matches!(
        response.status(),
        StatusCode::UNAUTHORIZED | StatusCode::NOT_FOUND | StatusCode::CONFLICT
    ) {
        return Ok(());
    }
    let _: AttemptStatusResponse = parse_json(response, "reconcile interrupted Scope run attempt")?;
    Ok(())
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

#[cfg(test)]
mod run_event_stream_tests {
    use super::*;
    use std::io::Cursor;

    #[test]
    fn sse_parser_delivers_log_events_and_honors_callback_stop() {
        let stream = concat!(
            ": keep-alive\n\n",
            "id: 7\n",
            "event: log\n",
            "data: {\"attempt_id\":\"attempt-1\",\"step_index\":0,\"position\":7,\"sequence\":2,",
            "\"text\":\"hello\\n\",\"created_at_unix\":9}\n\n",
            "event: error\n",
            "data: {\"message\":\"must not be reached\"}\n\n"
        );
        let mut logs = Vec::new();
        parse_run_event_stream(Cursor::new(stream), |event| {
            if let RunStreamEvent::Log(log) = event {
                logs.push(log);
            }
            Ok(false)
        })
        .unwrap();
        assert_eq!(logs.len(), 1);
        assert_eq!(logs[0].position, 7);
        assert_eq!(logs[0].text, "hello\n");
    }
}
