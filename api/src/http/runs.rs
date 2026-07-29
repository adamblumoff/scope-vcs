use crate::{
    auth::scope::require_scope_user, error::ApiError, http::responses::git_oid_request,
    persistence::unix_now, repo_access::find_repo,
    repo_cleanup::best_effort_cleanup_rollback_source_blobs, state::AppState,
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
use scope_api_contract::{CreateManualRunQuery, RunEventsQuery, RunLogResponse, RunResponse};
use scope_domain::{
    runs::{
        run::{Run, RunSource, RunTrigger},
        workflow::{RunnerSelector, WorkflowPath},
    },
    store::RepositoryActor,
};
use scope_object_store::{ContentObjectKind, put_content_object};
use std::{
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
const GIT_INSPECTION_TIMEOUT: Duration = Duration::from_secs(30);

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
    let desired_runner = match query.runner {
        Some(name) => RunnerSelector::named(name).map_err(ApiError::bad_request)?,
        None => revision.definition().runner().clone(),
    };
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
        desired_runner,
        now,
    )?;
    let run = match state.metadata.runs().enqueue_run(run, revision).await {
        Ok(run) => run,
        Err(error) => {
            best_effort_cleanup_rollback_source_blobs(&state, &[source_cleanup]).await;
            return Err(error.into());
        }
    };
    Ok(Json(run_response(&run)))
}

pub(crate) async fn get_run(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((owner, repo_name, run_id)): Path<(String, String, String)>,
) -> Result<Json<RunResponse>, ApiError> {
    let (run, _) = require_run_access(&state, &headers, &owner, &repo_name, &run_id).await?;
    Ok(Json(run_response(&run)))
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
    Ok(Json(run_response(&run)))
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
    Ok(Json(run_response(&run)))
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
        state, owner, repo_name, user_id, run_id, after, sender,
    ));
    Ok(Sse::new(ReceiverStream::new(receiver)).keep_alive(
        KeepAlive::new()
            .interval(Duration::from_secs(10))
            .text("keep-alive"),
    ))
}

async fn stream_run_events(
    state: AppState,
    owner: String,
    repo_name: String,
    user_id: String,
    run_id: String,
    mut cursor: u64,
    sender: tokio::sync::mpsc::Sender<Result<Event, Infallible>>,
) {
    let mut last_state = None;
    loop {
        if sender.is_closed() {
            return;
        }
        if let Err(error) = require_repo_member(&state, &user_id, &owner, &repo_name).await {
            send_stream_error(&sender, error.into_message()).await;
            return;
        }
        let logs = match state
            .metadata
            .runs()
            .run_logs_after(&run_id, cursor, RUN_LOG_STREAM_PAGE_SIZE as u64)
            .await
        {
            Ok(logs) => logs,
            Err(error) => {
                send_stream_error(&sender, error.message).await;
                return;
            }
        };
        let has_full_page = logs.len() == RUN_LOG_STREAM_PAGE_SIZE;
        for log in logs {
            cursor = log.position;
            let response = RunLogResponse {
                attempt_id: log.chunk.attempt_id,
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
                    send_stream_error(&sender, error.to_string()).await;
                    return;
                }
            };
            if sender.send(Ok(event)).await.is_err() {
                return;
            }
        }

        let run = match state.metadata.runs().run(&run_id).await {
            Ok(Some(run)) => run,
            Ok(None) => {
                send_stream_error(&sender, "run no longer exists".to_string()).await;
                return;
            }
            Err(error) => {
                send_stream_error(&sender, error.message).await;
                return;
            }
        };
        let terminal = run.state.is_terminal();
        if last_state != Some(run.state) && (!terminal || !has_full_page) {
            last_state = Some(run.state);
            let event = match Event::default()
                .event("status")
                .json_data(run_response(&run))
            {
                Ok(event) => event,
                Err(error) => {
                    send_stream_error(&sender, error.to_string()).await;
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
    message: String,
) {
    let data = serde_json::json!({ "message": message });
    if let Ok(event) = Event::default().event("error").json_data(data) {
        let _ = sender.send(Ok(event)).await;
    }
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

pub(crate) fn run_response(run: &Run) -> RunResponse {
    RunResponse {
        id: run.id.clone(),
        repository_id: run.workflow.repository_id().to_string(),
        workflow_name: run.workflow.path().name().to_string(),
        git_oid: run.source.git_oid().to_string(),
        desired_runner: match &run.desired_runner {
            RunnerSelector::Any => None,
            RunnerSelector::Named(name) => Some(name.clone()),
        },
        state: run.state,
        cancellation_requested: run.cancellation_requested,
        attempt_number: run.last_attempt_number,
        created_at_unix: run.created_at_unix,
        updated_at_unix: run.updated_at_unix,
        completed_at_unix: run.completed_at_unix,
    }
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
