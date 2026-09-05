use super::*;
use anyhow::Context;
use reqwest::blocking::Client;
use scope_api_contract::{
    CreateManualRunQuery, PushTriggerEvaluationResponse, RepositoryRunDetailResponse,
    ResolveManualRunResponse, RunEventsQuery, RunLogResponse, RunResponse,
};
use std::io::BufRead;

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

pub fn resolve_manual_run(
    client: &Client,
    api_url: &str,
    session_token: &str,
    owner: &str,
    repo: &str,
    query: &CreateManualRunQuery,
) -> anyhow::Result<ResolveManualRunResponse> {
    parse_json(
        client
            .post(format!(
                "{api_url}{}",
                routes::repo_run_resolve(owner, repo)
            ))
            .bearer_auth(session_token)
            .query(query)
            .send()
            .context("resolve Scope run source")?,
        "resolve Scope run source",
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

pub fn run_detail(
    client: &Client,
    api_url: &str,
    session_token: &str,
    owner: &str,
    repo: &str,
    run_id: &str,
) -> anyhow::Result<RepositoryRunDetailResponse> {
    parse_json(
        client
            .get(format!(
                "{api_url}{}",
                routes::repo_run_detail(owner, repo, run_id)
            ))
            .bearer_auth(session_token)
            .send()
            .context("load Scope run detail")?,
        "load Scope run detail",
    )
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
                        let error: ErrorResponse = serde_json::from_str(&payload)
                            .context("parse Scope run stream error")?;
                        return Err(crate::error::CliError::new(terminal_safe_error_response(
                            error,
                        ))
                        .into());
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

fn parse_json<T: serde::de::DeserializeOwned>(
    response: reqwest::blocking::Response,
    context: &str,
) -> anyhow::Result<T> {
    decode_json_response(response, context)
}

fn successful(
    response: reqwest::blocking::Response,
    context: &str,
) -> anyhow::Result<reqwest::blocking::Response> {
    successful_response(response, context)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::{CliError, ExitCategory};
    use std::io::Cursor;

    #[test]
    fn run_stream_errors_preserve_the_safe_diagnostic_reference() {
        let stream = concat!(
            "event: error\n",
            "data: {\"code\":\"internal\",\"message\":\"Scope hit an internal error.\",",
            "\"error_reference\":\"err_0123456789abcdef0123456789abcdef\",",
            "\"retryable\":false}\n\n",
        );

        let error = parse_run_event_stream(Cursor::new(stream), |_| Ok(true)).unwrap_err();
        let error = error.downcast_ref::<CliError>().expect("typed CLI error");

        assert_eq!(error.exit_category(), ExitCategory::Unexpected);
        assert_eq!(
            error.to_string(),
            "Scope hit an internal error.\nReference: err_0123456789abcdef0123456789abcdef"
        );
    }
}
