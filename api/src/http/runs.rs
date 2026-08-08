use crate::{
    auth::scope::require_scope_user,
    error::ApiError,
    http::responses::{
        RepositoryOperationsResponse, RepositoryRunAttemptResponse, RepositoryRunDetailResponse,
        RepositoryRunLogResponse, RepositoryRunStepLogPageResponse, RepositoryRunStepResponse,
        RepositoryRunnerResponse, RepositoryRunnerState, git_oid_request,
    },
    http::run_response::{repository_run_summary, run_response},
    persistence::unix_now,
    repo_access::find_repo,
    repo_cleanup::best_effort_cleanup_rollback_source_blobs,
    state::AppState,
};
use axum::{
    Json,
    body::{Body, to_bytes},
    extract::{Path, Query, State},
    http::HeaderMap,
    response::{
        IntoResponse,
        sse::{Event, KeepAlive, Sse},
    },
};
use scope_api_contract::{
    CreateManualRunQuery, PushTriggerCheckResponse, PushTriggerEvaluationResponse, RunEventsQuery,
    RunLogResponse, RunResponse,
};
use scope_domain::{
    runs::{
        run::{Run, RunSource, RunTrigger},
        workflow::{RunnerSelector, WorkflowPath},
    },
    store::RepositoryActor,
};
use scope_object_store::{ContentObjectKind, put_content_object};
use serde::Deserialize;
use std::{
    collections::BTreeMap,
    convert::Infallible,
    fs::{self, File, OpenOptions},
    io::Read,
    os::unix::{fs::DirBuilderExt, fs::OpenOptionsExt, process::CommandExt},
    path::{Path as FsPath, PathBuf},
    process::{Command, Stdio},
    thread,
    time::{Duration, Instant},
};
use tokio_stream::wrappers::ReceiverStream;

const MAX_MANUAL_BUNDLE_BYTES: usize = 128 * 1024 * 1024;
const RUN_LOG_STREAM_PAGE_SIZE: usize = 64;
const RUN_LOG_STREAM_BUFFER: usize = 32;
const RUN_LOG_STREAM_AUTH_RECHECK: Duration = Duration::from_secs(30);
const GIT_INSPECTION_TIMEOUT: Duration = Duration::from_secs(30);
const REPOSITORY_RUN_LIMIT: u64 = 20;
const REPOSITORY_STEP_LOG_LIMIT: u64 = 128;
const RUNNER_ONLINE_WINDOW_SECONDS: u64 = 90;

pub(crate) async fn create_manual_run(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((owner, repo_name)): Path<(String, String)>,
    Query(query): Query<CreateManualRunQuery>,
    body: Body,
) -> Result<Json<RunResponse>, ApiError> {
    let user = require_scope_user(&state, &headers).await?;
    let repo = require_repo_member(&state, &user.id, &owner, &repo_name).await?;
    validate_request_id(&query.request_id)?;
    let git_oid = git_oid_request("git_oid", &query.git_oid)?;
    let bytes = to_bytes(body, MAX_MANUAL_BUNDLE_BYTES)
        .await
        .map_err(|error| ApiError::payload_too_large(format!("run bundle is too large: {error}")))?
        .to_vec();
    let inspect_root = state.data_dir.join("run-bundle-inspection");
    let workflow_name = query.workflow.clone();
    let inspected = tokio::task::spawn_blocking(move || {
        inspect_bundle(&inspect_root, &bytes, &git_oid, &workflow_name)
            .map(|workflow| (bytes, git_oid, workflow))
    })
    .await
    .map_err(|error| {
        ApiError::internal_message(format!("run bundle inspection failed: {error}"))
    })??;
    let (bytes, git_oid, parsed_workflow) = inspected;
    let revision = parsed_workflow
        .into_revision(repo.record.id.clone())
        .map_err(ApiError::bad_request)?;
    if !revision.definition().triggers().manual() {
        return Err(ApiError::bad_request(
            "workflow does not enable the manual trigger",
        ));
    }
    let runner_override = query
        .runner
        .map(RunnerSelector::named)
        .transpose()
        .map_err(ApiError::bad_request)?;
    let mut stored = put_content_object(
        state.object_store.as_ref(),
        ContentObjectKind::GitBundle,
        &bytes,
    )?;
    stored.git_oid = git_oid;
    let now = unix_now()?;
    let source_cleanup = stored.clone();
    let run = Run::new(
        format!("run_{}", query.request_id),
        format!("manual:{}", query.request_id),
        revision.workflow().clone(),
        revision.digest(),
        RunTrigger::Manual,
        Some(user.id),
        RunSource::ephemeral_git_bundle(stored)?,
        runner_override,
        now,
    )?;
    let run = match state.metadata.runs().enqueue_run(run, revision).await {
        Ok(run) => run,
        Err(error) => {
            best_effort_cleanup_rollback_source_blobs(&state, &[source_cleanup]).await;
            return Err(error.into());
        }
    };
    let jobs = state.metadata.runs().run_jobs(&run.id).await?;
    Ok(Json(run_response(&run, &jobs, false)?))
}

pub(crate) async fn get_run(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((owner, repo_name, run_id)): Path<(String, String, String)>,
) -> Result<Json<RunResponse>, ApiError> {
    require_run_access(&state, &headers, &owner, &repo_name, &run_id).await?;
    let snapshot = state
        .metadata
        .runs()
        .run_snapshot(&run_id)
        .await?
        .ok_or_else(|| ApiError::not_found("run not found"))?;
    Ok(Json(run_response(
        &snapshot.run,
        &snapshot.jobs,
        snapshot.logs_truncated,
    )?))
}

pub(crate) async fn get_repository_operations(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((owner, repo_name)): Path<(String, String)>,
) -> Result<Json<RepositoryOperationsResponse>, ApiError> {
    let user = require_scope_user(&state, &headers).await?;
    let repo = require_repo_member(&state, &user.id, &owner, &repo_name).await?;
    let runs_store = state.metadata.runs();
    let (runs, runners) = tokio::try_join!(
        runs_store.repository_operations_runs(&repo.record.id, REPOSITORY_RUN_LIMIT),
        runs_store.repository_runners(&repo.record.id),
    )?;
    let now_unix = unix_now()?;

    let runs = runs
        .iter()
        .map(|entry| repository_run_summary(&entry.run, &entry.jobs))
        .collect::<Result<Vec<_>, ApiError>>()?;
    Ok(Json(RepositoryOperationsResponse {
        runs,
        runners: runners
            .into_iter()
            .map(|entry| {
                let state = if !entry.runner.supports_dispatch() {
                    RepositoryRunnerState::Disabled
                } else if entry.runner.last_seen_at_unix.is_some_and(|last_seen| {
                    last_seen >= now_unix.saturating_sub(RUNNER_ONLINE_WINDOW_SECONDS)
                }) {
                    RepositoryRunnerState::Online
                } else {
                    RepositoryRunnerState::Offline
                };
                RepositoryRunnerResponse {
                    id: entry.runner.id,
                    name: entry.grant.name.as_str().to_string(),
                    version: entry.runner.version,
                    state,
                    last_seen_at_unix: entry.runner.last_seen_at_unix,
                }
            })
            .collect(),
    }))
}

pub(crate) async fn get_repository_run_detail(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((owner, repo_name, run_id)): Path<(String, String, String)>,
) -> Result<Json<RepositoryRunDetailResponse>, ApiError> {
    require_run_access(&state, &headers, &owner, &repo_name, &run_id).await?;
    let runs_store = state.metadata.runs();
    let detail = runs_store
        .run_detail(&run_id)
        .await?
        .ok_or_else(|| ApiError::not_found("run not found"))?;
    let workflow = detail.workflow_revision.definition();
    let attempts = detail
        .attempts
        .into_iter()
        .map(|detail| {
            let attempt = detail.attempt;
            let workflow_steps = workflow
                .job(&attempt.job_key)
                .ok_or_else(|| {
                    ApiError::internal_message(
                        "persisted run attempt job is missing from its workflow revision",
                    )
                })?
                .steps();
            let steps = detail
                .steps
                .into_iter()
                .map(|step| {
                    let definition =
                        workflow_steps
                            .get(step.step_index as usize)
                            .ok_or_else(|| {
                                ApiError::internal_message(
                                    "persisted run step is missing from its workflow revision",
                                )
                            })?;
                    Ok(RepositoryRunStepResponse {
                        index: step.step_index,
                        name: definition.name().to_string(),
                        command: definition.run().to_string(),
                        state: step.state.into(),
                        started_at_unix: step.started_at_unix,
                        completed_at_unix: step.completed_at_unix,
                        exit_code: step.exit_code,
                    })
                })
                .collect::<Result<Vec<_>, ApiError>>()?;
            Ok(RepositoryRunAttemptResponse {
                id: attempt.id,
                job_key: attempt.job_key.as_str().to_string(),
                runner_id: attempt.runner_id,
                runner_name: attempt.runner_name,
                state: attempt.state.into(),
                created_at_unix: attempt.created_at_unix,
                started_at_unix: attempt.started_at_unix,
                completed_at_unix: attempt.completed_at_unix,
                terminal_reason: attempt.terminal_reason.map(Into::into),
                steps,
            })
        })
        .collect::<Result<Vec<_>, ApiError>>()?;

    Ok(Json(RepositoryRunDetailResponse {
        run: repository_run_summary(&detail.run, &detail.jobs)?,
        attempts,
    }))
}

#[derive(Debug, Deserialize)]
pub(crate) struct RepositoryStepLogsQuery {
    #[serde(default)]
    after: u64,
}

pub(crate) async fn get_repository_run_step_logs(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((owner, repo_name, run_id, attempt_id, step_index)): Path<(
        String,
        String,
        String,
        String,
        u32,
    )>,
    Query(query): Query<RepositoryStepLogsQuery>,
) -> Result<Json<RepositoryRunStepLogPageResponse>, ApiError> {
    require_run_access(&state, &headers, &owner, &repo_name, &run_id).await?;
    let page = state
        .metadata
        .runs()
        .attempt_step_logs_after(
            &run_id,
            &attempt_id,
            step_index,
            query.after,
            REPOSITORY_STEP_LOG_LIMIT,
        )
        .await?;
    let next_after = page
        .logs
        .last()
        .map_or(query.after, |stored| stored.position);

    Ok(Json(RepositoryRunStepLogPageResponse {
        logs: page
            .logs
            .into_iter()
            .map(|stored| RepositoryRunLogResponse {
                position: stored.position,
                sequence: stored.chunk.sequence,
                text: stored.chunk.text,
                created_at_unix: stored.chunk.created_at_unix,
            })
            .collect(),
        next_after,
        logs_truncated: page.logs_truncated,
    }))
}

pub(crate) async fn get_push_trigger_evaluation(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((owner, repo_name, head_oid)): Path<(String, String, String)>,
) -> Result<Json<PushTriggerEvaluationResponse>, ApiError> {
    let user = require_scope_user(&state, &headers).await?;
    let repo = require_repo_member(&state, &user.id, &owner, &repo_name).await?;
    let head_oid = git_oid_request("head_oid", &head_oid)?;
    let evaluation = state
        .metadata
        .runs()
        .push_trigger_evaluation(&repo.record.id, &head_oid)
        .await?
        .ok_or_else(|| ApiError::not_found("push trigger evaluation not found"))?;
    let run_ids = evaluation
        .checks
        .iter()
        .map(|check| check.run_id.clone())
        .collect::<Vec<_>>();
    let runs_store = state.metadata.runs();
    let (runs, mut jobs, truncated_run_ids) = tokio::try_join!(
        runs_store.runs_by_ids(&run_ids),
        runs_store.run_jobs_by_ids(&run_ids),
        runs_store.run_ids_with_truncated_logs(&run_ids),
    )?;
    let mut runs = runs
        .into_iter()
        .map(|run| (run.id.clone(), run))
        .collect::<BTreeMap<_, _>>();
    if run_ids.iter().any(|run_id| !runs.contains_key(run_id)) {
        return Err(ApiError::not_found(
            "push trigger evaluation history has expired",
        ));
    }
    let checks = evaluation
        .checks
        .into_iter()
        .map(|check| {
            let run = runs
                .remove(&check.run_id)
                .ok_or_else(|| ApiError::internal_message("push trigger check run is missing"))?;
            let run_jobs = jobs
                .remove(&run.id)
                .filter(|jobs| !jobs.is_empty())
                .ok_or_else(|| {
                    ApiError::internal_message(
                        "push trigger check run is missing its persisted jobs",
                    )
                })?;
            Ok(PushTriggerCheckResponse {
                workflow_path: check.workflow_path,
                workflow_name: check.workflow_name,
                run: run_response(&run, &run_jobs, truncated_run_ids.contains(&run.id))?,
            })
        })
        .collect::<Result<Vec<_>, ApiError>>()?;
    Ok(Json(PushTriggerEvaluationResponse {
        change_version: evaluation.change_version,
        head_oid: evaluation.head_oid,
        state: evaluation.state,
        message: evaluation.message,
        checks,
    }))
}

pub(crate) async fn cancel_run(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((owner, repo_name, run_id)): Path<(String, String, String)>,
) -> Result<Json<RunResponse>, ApiError> {
    require_run_access(&state, &headers, &owner, &repo_name, &run_id).await?;
    let run = state
        .metadata
        .runs()
        .request_run_cancellation(&run_id, unix_now()?)
        .await?;
    let logs_truncated = state
        .metadata
        .runs()
        .run_has_truncated_logs(&run_id)
        .await?;
    let jobs = state.metadata.runs().run_jobs(&run.id).await?;
    Ok(Json(run_response(&run, &jobs, logs_truncated)?))
}

pub(crate) async fn retry_run(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((owner, repo_name, run_id)): Path<(String, String, String)>,
) -> Result<Json<RunResponse>, ApiError> {
    require_run_access(&state, &headers, &owner, &repo_name, &run_id).await?;
    let run = state
        .metadata
        .runs()
        .retry_run(&run_id, unix_now()?)
        .await?;
    let logs_truncated = state
        .metadata
        .runs()
        .run_has_truncated_logs(&run_id)
        .await?;
    let jobs = state.metadata.runs().run_jobs(&run.id).await?;
    Ok(Json(run_response(&run, &jobs, logs_truncated)?))
}

pub(crate) async fn run_events(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((owner, repo_name, run_id)): Path<(String, String, String)>,
    Query(query): Query<RunEventsQuery>,
) -> Result<impl IntoResponse, ApiError> {
    let (_, user_id) = require_run_access(&state, &headers, &owner, &repo_name, &run_id).await?;
    let after = match headers.get("last-event-id") {
        Some(value) => value
            .to_str()
            .map_err(|_| ApiError::bad_request("last-event-id must be an integer"))?
            .parse::<u64>()
            .map_err(|_| ApiError::bad_request("last-event-id must be an integer"))?
            .max(query.after),
        None => query.after,
    };
    let (sender, receiver) = tokio::sync::mpsc::channel(RUN_LOG_STREAM_BUFFER);
    tokio::spawn(stream_run_events(
        RunEventStreamContext {
            state,
            headers,
            owner,
            repo_name,
            user_id,
            run_id,
        },
        after,
        sender,
    ));
    Ok(Sse::new(ReceiverStream::new(receiver)).keep_alive(
        KeepAlive::new()
            .interval(Duration::from_secs(10))
            .text("keep-alive"),
    ))
}

struct RunEventStreamContext {
    state: AppState,
    headers: HeaderMap,
    owner: String,
    repo_name: String,
    user_id: String,
    run_id: String,
}

async fn stream_run_events(
    context: RunEventStreamContext,
    mut cursor: u64,
    sender: tokio::sync::mpsc::Sender<Result<Event, Infallible>>,
) {
    let mut last_state = None;
    let mut terminal_observed = false;
    let mut authenticated_at = Instant::now();
    loop {
        if sender.is_closed() {
            return;
        }
        if authenticated_at.elapsed() >= RUN_LOG_STREAM_AUTH_RECHECK {
            match require_scope_user(&context.state, &context.headers).await {
                Ok(user) if user.id == context.user_id => authenticated_at = Instant::now(),
                Ok(_) => {
                    send_stream_error(&sender, ApiError::forbidden("run access changed")).await;
                    return;
                }
                Err(error) => {
                    send_stream_error(&sender, error).await;
                    return;
                }
            }
        }
        if let Err(error) = require_repo_member(
            &context.state,
            &context.user_id,
            &context.owner,
            &context.repo_name,
        )
        .await
        {
            send_stream_error(&sender, error).await;
            return;
        }
        let logs = match context
            .state
            .metadata
            .runs()
            .run_logs_after(&context.run_id, cursor, RUN_LOG_STREAM_PAGE_SIZE as u64)
            .await
        {
            Ok(logs) => logs,
            Err(error) => {
                send_stream_error(&sender, error.into()).await;
                return;
            }
        };
        let has_full_page = logs.len() == RUN_LOG_STREAM_PAGE_SIZE;
        for log in logs {
            cursor = log.position;
            let response = RunLogResponse {
                attempt_id: log.chunk.attempt_id,
                step_index: log.chunk.step_index,
                position: log.position,
                sequence: log.chunk.sequence,
                text: log.chunk.text,
                created_at_unix: log.chunk.created_at_unix,
            };
            let event = match Event::default()
                .event("log")
                .id(cursor.to_string())
                .json_data(response)
            {
                Ok(event) => event,
                Err(error) => {
                    send_stream_error(&sender, ApiError::internal(error)).await;
                    return;
                }
            };
            if sender.send(Ok(event)).await.is_err() {
                return;
            }
        }

        let snapshot = match context
            .state
            .metadata
            .runs()
            .run_snapshot(&context.run_id)
            .await
        {
            Ok(Some(snapshot)) => snapshot,
            Ok(None) => {
                send_stream_error(&sender, ApiError::not_found("run no longer exists")).await;
                return;
            }
            Err(error) => {
                send_stream_error(&sender, error.into()).await;
                return;
            }
        };
        let run = &snapshot.run;
        let terminal = run.state.is_terminal();
        if terminal && !terminal_observed {
            // Log append and attempt completion are separate transactions. Seeing the terminal
            // state makes completion a stable watermark because terminal attempts reject later
            // logs; read once more before closing so a log committed between the two queries
            // above cannot be omitted.
            terminal_observed = true;
            continue;
        }
        if last_state != Some(run.state) && (!terminal || !has_full_page) {
            last_state = Some(run.state);
            let response = match run_response(run, &snapshot.jobs, snapshot.logs_truncated) {
                Ok(response) => response,
                Err(error) => {
                    send_stream_error(&sender, error).await;
                    return;
                }
            };
            let event = match Event::default().event("status").json_data(response) {
                Ok(event) => event,
                Err(error) => {
                    send_stream_error(&sender, ApiError::internal(error)).await;
                    return;
                }
            };
            if sender.send(Ok(event)).await.is_err() {
                return;
            }
        }
        if terminal && !has_full_page {
            return;
        }
        if has_full_page {
            continue;
        }
        tokio::select! {
            () = sender.closed() => return,
            () = tokio::time::sleep(Duration::from_secs(1)) => {}
        }
    }
}

async fn send_stream_error(
    sender: &tokio::sync::mpsc::Sender<Result<Event, Infallible>>,
    error: ApiError,
) {
    let data = stream_error_data(error);
    if let Ok(event) = Event::default().event("error").json_data(data) {
        let _ = sender.send(Ok(event)).await;
    }
}

fn stream_error_data(error: ApiError) -> serde_json::Value {
    serde_json::json!({ "message": error.into_public_message() })
}

async fn require_run_access(
    state: &AppState,
    headers: &HeaderMap,
    owner: &str,
    repo_name: &str,
    run_id: &str,
) -> Result<(Run, String), ApiError> {
    let user = require_scope_user(state, headers).await?;
    let repo = require_repo_member(state, &user.id, owner, repo_name).await?;
    let run = state
        .metadata
        .runs()
        .run(run_id)
        .await?
        .ok_or_else(|| ApiError::not_found("run not found"))?;
    if run.workflow.repository_id() != repo.record.id {
        return Err(ApiError::not_found("run not found"));
    }
    Ok((run, user.id))
}

async fn require_repo_member(
    state: &AppState,
    user_id: &str,
    owner: &str,
    name: &str,
) -> Result<scope_domain::store::StoredRepository, ApiError> {
    let repo = find_repo(state, owner, name).await?;
    if repo.access_for_user_id(user_id).actor == RepositoryActor::Public {
        return Err(ApiError::forbidden("repo membership required"));
    }
    Ok(repo)
}

fn validate_request_id(request_id: &str) -> Result<(), ApiError> {
    if request_id.len() != 32 || !request_id.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(ApiError::bad_request(
            "request_id must be a 32-character hexadecimal value",
        ));
    }
    Ok(())
}

fn inspect_bundle(
    root: &FsPath,
    bytes: &[u8],
    git_oid: &str,
    workflow_name: &str,
) -> Result<scope_run_config::ParsedWorkflow, ApiError> {
    let yml = WorkflowPath::parse(format!("/.scope/runs/{workflow_name}.yml"))
        .map_err(ApiError::bad_request)?;
    let yaml = WorkflowPath::parse(format!("/.scope/runs/{workflow_name}.yaml"))
        .map_err(ApiError::bad_request)?;
    let temp = RunTempDir::new(root)?;
    let bundle = temp.path.join("source.bundle");
    let bare = temp.path.join("source.git");
    write_private_file(&bundle, bytes)?;
    let mut clone = Command::new("git");
    clone
        .args(["clone", "--bare", "--no-local"])
        .arg(&bundle)
        .arg(&bare)
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    if !run_git_with_timeout(&mut clone, "Git bundle clone")? {
        return Err(ApiError::bad_request("invalid Git bundle"));
    }
    let mut commit = Command::new("git");
    commit
        .arg("--git-dir")
        .arg(&bare)
        .args(["cat-file", "-e", &format!("{git_oid}^{{commit}}")])
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    if !run_git_with_timeout(&mut commit, "Git commit inspection")? {
        return Err(ApiError::bad_request(
            "requested Git commit is not present in the bundle",
        ));
    }
    let yml_bytes = git_blob(
        &bare,
        git_oid,
        yml.as_str().trim_start_matches('/'),
        &temp.path.join("workflow-yml"),
    )?;
    let yaml_bytes = git_blob(
        &bare,
        git_oid,
        yaml.as_str().trim_start_matches('/'),
        &temp.path.join("workflow-yaml"),
    )?;
    let (path, workflow_bytes) = match (yml_bytes, yaml_bytes) {
        (Some(_), Some(_)) => {
            return Err(ApiError::bad_request(format!(
                "workflow {workflow_name:?} is defined by both .yml and .yaml"
            )));
        }
        (Some(bytes), None) => (yml, bytes),
        (None, Some(bytes)) => (yaml, bytes),
        (None, None) => {
            return Err(ApiError::not_found(format!(
                "workflow {workflow_name:?} was not found at commit {git_oid}"
            )));
        }
    };
    scope_run_config::parse_workflow(path.as_str(), &workflow_bytes).map_err(ApiError::bad_request)
}

fn git_blob(
    bare: &FsPath,
    git_oid: &str,
    path: &str,
    output_prefix: &FsPath,
) -> Result<Option<Vec<u8>>, ApiError> {
    let object = format!("{git_oid}:{path}");
    let size_path = output_prefix.with_extension("size");
    let size_file = create_private_file(&size_path)?;
    let mut size = Command::new("git");
    size.arg("--git-dir")
        .arg(bare)
        .args(["cat-file", "-s", &object])
        .stdout(Stdio::from(size_file))
        .stderr(Stdio::null());
    if !run_git_with_timeout(&mut size, "Git workflow size inspection")? {
        return Ok(None);
    }
    let size_text = read_bounded_file(&size_path, 64)?;
    let size = std::str::from_utf8(&size_text)
        .map_err(|_| ApiError::bad_request("Git reported an invalid workflow size"))?
        .trim()
        .parse::<usize>()
        .map_err(|_| ApiError::bad_request("Git reported an invalid workflow size"))?;
    if size > scope_run_config::MAX_WORKFLOW_DEFINITION_BYTES {
        return Err(ApiError::bad_request(format!(
            "workflow definition exceeds {} bytes",
            scope_run_config::MAX_WORKFLOW_DEFINITION_BYTES
        )));
    }

    let blob_path = output_prefix.with_extension("blob");
    let blob_file = create_private_file(&blob_path)?;
    let mut blob = Command::new("git");
    blob.arg("--git-dir")
        .arg(bare)
        .args(["cat-file", "blob", &object])
        .stdout(Stdio::from(blob_file))
        .stderr(Stdio::null());
    if !run_git_with_timeout(&mut blob, "Git workflow read")? {
        return Err(ApiError::bad_request(
            "workflow changed while inspecting the Git bundle",
        ));
    }
    let bytes = read_bounded_file(&blob_path, scope_run_config::MAX_WORKFLOW_DEFINITION_BYTES)?;
    if bytes.len() != size {
        return Err(ApiError::bad_request(
            "Git workflow size changed while inspecting the bundle",
        ));
    }
    Ok(Some(bytes))
}

fn run_git_with_timeout(command: &mut Command, operation: &str) -> Result<bool, ApiError> {
    command.process_group(0);
    let mut child = command.spawn().map_err(ApiError::internal)?;
    let started = Instant::now();
    loop {
        if let Some(status) = child.try_wait().map_err(ApiError::internal)? {
            return Ok(status.success());
        }
        if started.elapsed() >= GIT_INSPECTION_TIMEOUT {
            let _ = Command::new("kill")
                .args(["-KILL", "--"])
                .arg(format!("-{}", child.id()))
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status();
            let _ = child.kill();
            let _ = child.wait();
            return Err(ApiError::bad_request(format!("{operation} timed out")));
        }
        thread::sleep(Duration::from_millis(25));
    }
}

fn write_private_file(path: &FsPath, bytes: &[u8]) -> Result<(), ApiError> {
    let mut file = create_private_file(path)?;
    std::io::Write::write_all(&mut file, bytes).map_err(ApiError::internal)
}

fn create_private_file(path: &FsPath) -> Result<File, ApiError> {
    OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(path)
        .map_err(ApiError::internal)
}

fn read_bounded_file(path: &FsPath, max_bytes: usize) -> Result<Vec<u8>, ApiError> {
    let file = File::open(path).map_err(ApiError::internal)?;
    let length = file.metadata().map_err(ApiError::internal)?.len();
    if length > max_bytes as u64 {
        return Err(ApiError::bad_request(format!(
            "Git command output exceeds {max_bytes} bytes"
        )));
    }
    let mut bytes = Vec::with_capacity(length as usize);
    file.take(max_bytes as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(ApiError::internal)?;
    if bytes.len() > max_bytes {
        return Err(ApiError::bad_request(format!(
            "Git command output exceeds {max_bytes} bytes"
        )));
    }
    Ok(bytes)
}

struct RunTempDir {
    path: PathBuf,
}

impl RunTempDir {
    fn new(root: &FsPath) -> Result<Self, ApiError> {
        fs::create_dir_all(root).map_err(ApiError::internal)?;
        for _ in 0..8 {
            let path = root.join(crate::persistence_ids::generate_prefixed_id("inspect_")?);
            match fs::DirBuilder::new().mode(0o700).create(&path) {
                Ok(()) => return Ok(Self { path }),
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
                Err(error) => return Err(ApiError::internal(error)),
            }
        }
        Err(ApiError::internal_message(
            "could not allocate run bundle inspection directory",
        ))
    }
}

impl Drop for RunTempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use scope_api_contract::RunRunnerSelection;
    use scope_domain::{
        content_ref::ContentRef,
        runs::{
            job::RunJob,
            run::{MAX_RUN_ATTEMPTS, RunJobState, RunState},
            workflow::{WorkflowIdentity, WorkflowJobId},
        },
        store::SourceBlob,
    };

    #[test]
    fn stream_errors_redact_database_diagnostics() {
        let diagnostic = "relation scope_runs_internal does not exist";
        let error = scope_postgres::error::PostgresError::internal_message(diagnostic);

        let data = stream_error_data(error.into());
        let message = data["message"].as_str().unwrap();

        assert!(message.starts_with("Scope hit an internal error. (reference: err_"));
        assert!(!message.contains(diagnostic));
    }

    #[test]
    fn run_summary_allows_retry_when_every_job_has_capacity() {
        let run = terminal_run();
        let available = terminal_job(1);
        assert!(
            repository_run_summary(&run, &[available])
                .unwrap()
                .can_retry
        );
    }

    #[test]
    fn run_summary_hides_retry_when_any_job_is_exhausted() {
        let run = terminal_run();
        let exhausted = terminal_job(MAX_RUN_ATTEMPTS);
        assert!(
            !repository_run_summary(&run, &[exhausted])
                .unwrap()
                .can_retry
        );
    }

    #[test]
    fn run_responses_report_effective_named_and_mixed_runner_selection() {
        let run = terminal_run();
        let mut linux_one = terminal_job(1);
        linux_one.desired_runner = RunnerSelector::named("linux-one").unwrap();
        assert_eq!(
            run_response(&run, &[linux_one.clone()], false)
                .unwrap()
                .runner_selection,
            RunRunnerSelection::Named {
                name: "linux-one".to_string()
            }
        );

        let mut linux_two = terminal_job(1);
        linux_two.key = WorkflowJobId::parse("lint").unwrap();
        linux_two.desired_runner = RunnerSelector::named("linux-two").unwrap();
        assert_eq!(
            repository_run_summary(&run, &[linux_one, linux_two])
                .unwrap()
                .runner_selection,
            RunRunnerSelection::Mixed
        );
    }

    fn terminal_run() -> Run {
        Run::restore(
            "run-summary",
            "manual:summary",
            WorkflowIdentity::new(
                "owner/repo",
                WorkflowPath::parse("/.scope/runs/checks.yml").unwrap(),
            )
            .unwrap(),
            "a".repeat(64),
            RunTrigger::Manual,
            Some("user-1".to_string()),
            RunSource::ephemeral_git_bundle(SourceBlob {
                content_ref: ContentRef::git_bundle_sha256("b".repeat(64)),
                sha256: "b".repeat(64),
                git_oid: "c".repeat(40),
                git_file_mode: "100644".to_string(),
                size_bytes: 1,
            })
            .unwrap(),
            None,
            RunState::Failed,
            false,
            1,
            2,
            Some(2),
        )
        .unwrap()
    }

    fn terminal_job(last_attempt_number: u32) -> RunJob {
        RunJob::restore(
            "run-summary",
            WorkflowJobId::parse("checks").unwrap(),
            RunnerSelector::Any,
            None,
            RunJobState::Failed,
            last_attempt_number,
            None,
            1,
            2,
            Some(2),
        )
        .unwrap()
    }
}
