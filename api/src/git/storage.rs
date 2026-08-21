use crate::{
    config::{DEFAULT_GIT_BRANCH, EMPTY_GIT_OID, RECEIVE_PACK_STAGING_BYTES},
    error::ApiError,
    git::import::{run_git, safe_repo_key},
    git::projection_repo::projection_bare_repo_for_state,
    git::upload::git_command_output_with_timeout,
    persistence::ensure_private_dir,
    repo_access::find_repo,
    runtime_budgets::RuntimeBudgets,
    state::AppState,
};
use axum::{body::Body, http::StatusCode, response::Response};
use futures_util::StreamExt;
use scope_domain::policy::Principal;
use scope_domain::projection::{ProjectionViewKey, project_graph};
use scope_domain::store::{
    GitHead, GitPackSpan, RepoLifecycleState, SourceBlob, validate_git_pack_layout,
};
use scope_object_store::source_blob_bytes;
use sha2::{Digest, Sha256};
use std::time::Instant;
use std::{
    fs,
    path::{Path as FsPath, PathBuf},
    process::{Command, Stdio},
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    task::JoinHandle,
};

pub(crate) fn receive_pack_staging_repo_path(
    state: &AppState,
    owner: &str,
    repo_name: &str,
) -> Result<PathBuf, ApiError> {
    let mut bytes = [0_u8; RECEIVE_PACK_STAGING_BYTES];
    getrandom::fill(&mut bytes).map_err(|error| {
        ApiError::internal_message(format!(
            "failed to create receive-pack staging path: {error}"
        ))
    })?;
    let base_dir = state.data_dir.as_ref().clone();
    let repo_id = scope_domain::store::repo_id(owner, repo_name);
    let digest = Sha256::digest(repo_id.as_bytes());
    let digest = hex::encode(digest);
    ensure_private_dir(&base_dir)?;
    Ok(base_dir
        .join("git-rx")
        .join(format!("{}-{}.git", &digest[..16], hex::encode(bytes))))
}

pub(crate) fn receive_pack_staging_repo_prefix(owner: &str, repo_name: &str) -> String {
    let repo_id = scope_domain::store::repo_id(owner, repo_name);
    let digest = Sha256::digest(repo_id.as_bytes());
    let digest = hex::encode(digest);
    digest[..16].to_string()
}

pub(crate) fn owner_git_repo_path(state: &AppState, owner: &str, repo_name: &str) -> PathBuf {
    git_repo_storage_root(state)
        .join("git-repos")
        .join(format!("{}.git", safe_repo_key(owner, repo_name)))
}

pub(crate) fn staged_git_repo_path(state: &AppState, owner: &str, repo_name: &str) -> PathBuf {
    git_repo_storage_root(state)
        .join("git-staged")
        .join(format!("{}.git", safe_repo_key(owner, repo_name)))
}

pub(crate) fn request_ref_store_repo_path(
    state: &AppState,
    owner: &str,
    repo_name: &str,
) -> PathBuf {
    git_repo_storage_root(state)
        .join("git-request-refs")
        .join(format!("{}.git", safe_repo_key(owner, repo_name)))
}

pub(crate) fn git_repo_storage_root(state: &AppState) -> PathBuf {
    state.data_dir.as_ref().clone()
}

pub(crate) fn delete_repo_storage(
    state: &AppState,
    owner: &str,
    repo_name: &str,
) -> Result<(), ApiError> {
    remove_dir_if_exists(
        &state
            .repository_engine
            .repository_path(&scope_domain::store::repo_id(owner, repo_name)),
    )?;
    remove_dir_if_exists(&owner_git_repo_path(state, owner, repo_name))?;
    remove_dir_if_exists(&staged_git_repo_path(state, owner, repo_name))?;
    remove_dir_if_exists(&request_ref_store_repo_path(state, owner, repo_name))?;
    delete_request_ref_locks(state, owner, repo_name)?;

    let rx_root = git_repo_storage_root(state).join("git-rx");
    let prefix = receive_pack_staging_repo_prefix(owner, repo_name);
    let entries = match fs::read_dir(&rx_root) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(ApiError::internal(error)),
    };
    for entry in entries {
        let entry = entry.map_err(ApiError::internal)?;
        let file_name = entry.file_name();
        if file_name.to_string_lossy().starts_with(&prefix) {
            remove_dir_if_exists(&entry.path())?;
        }
    }

    Ok(())
}

fn delete_request_ref_locks(
    state: &AppState,
    owner: &str,
    repo_name: &str,
) -> Result<(), ApiError> {
    let lock_root = git_repo_storage_root(state).join("git-request-refs-locks");
    let entries = match fs::read_dir(&lock_root) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(ApiError::internal(error)),
    };
    let prefix = format!("{}-", safe_repo_key(owner, repo_name));
    for entry in entries {
        let entry = entry.map_err(ApiError::internal)?;
        let file_name = entry.file_name();
        if file_name.to_string_lossy().starts_with(&prefix) {
            match fs::remove_file(entry.path()) {
                Ok(()) => {}
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => return Err(ApiError::internal(error)),
            }
        }
    }
    Ok(())
}

pub(crate) fn remove_dir_if_exists(path: &FsPath) -> Result<(), ApiError> {
    match fs::remove_dir_all(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(ApiError::internal(error)),
    }
}

pub(crate) fn ensure_first_push_receive_pack_staging_repo(
    state: &AppState,
    owner: &str,
    repo_name: &str,
) -> Result<PathBuf, ApiError> {
    let repo_root = receive_pack_staging_repo_path(state, owner, repo_name)?;
    if let Some(parent) = repo_root.parent() {
        ensure_private_dir(parent)?;
    }
    run_git(
        None,
        &["init", "--bare", repo_root.to_string_lossy().as_ref()],
        "initializing receive-pack staging repo",
    )?;
    run_git(
        Some(&repo_root),
        &["config", "http.receivepack", "true"],
        "enabling receive-pack",
    )?;
    run_git(
        Some(&repo_root),
        &[
            "symbolic-ref",
            "HEAD",
            &format!("refs/heads/{DEFAULT_GIT_BRANCH}"),
        ],
        "setting receive-pack default branch",
    )?;
    install_first_push_pre_receive_hook(&repo_root)?;
    Ok(repo_root)
}

pub(crate) async fn ensure_ready_receive_pack_staging_repo(
    state: &AppState,
    owner: &str,
    repo_name: &str,
    author_id: &str,
) -> Result<PathBuf, ApiError> {
    let repo = state
        .metadata
        .repositories()
        .git_push_context(owner, repo_name, author_id)
        .await?
        .ok_or_else(|| ApiError::not_found(format!("repo {owner}/{repo_name} not found")))?;
    if repo.lifecycle_state != RepoLifecycleState::Ready {
        return Err(ApiError::conflict("repo must be ready before push"));
    }
    let repo_root = receive_pack_staging_repo_path(state, owner, repo_name)?;
    if let Some(parent) = repo_root.parent() {
        ensure_private_dir(parent)?;
    }
    if let Some(head) = repo.git_head.as_ref() {
        let seed_repo = state.repository_engine.materialize_repository(
            state,
            &repo.repo_id,
            head,
            &repo.git_pack_spans,
        )?;
        let seed = seed_repo.to_string_lossy().to_string();
        let target = repo_root.to_string_lossy().to_string();
        run_git(
            None,
            &["clone", "--bare", "--local", &seed, &target],
            "cloning receive-pack staging repo",
        )?;
    } else {
        let repo = find_repo(state, owner, repo_name).await?;
        let principal = Principal {
            id: author_id.to_string(),
            kind: scope_domain::policy::PrincipalKind::User,
        };
        let view_key = ProjectionViewKey::from_access(repo.access_for_principal(&principal));
        let projection = project_graph(&repo.graph, &repo.visibility_change_sets, view_key);
        let seed_repo = projection_bare_repo_for_state(
            state,
            &repo.graph.repo_id,
            &projection,
            repo.git_head.as_ref(),
            &repo.git_pack_spans,
        )?;
        let seed = seed_repo.to_string_lossy().to_string();
        let target = repo_root.to_string_lossy().to_string();
        run_git(
            None,
            &["clone", "--bare", "--shared", &seed, &target],
            "cloning receive-pack staging repo",
        )?;
    }
    run_git(
        Some(&repo_root),
        &["config", "http.receivepack", "true"],
        "enabling receive-pack",
    )?;
    install_ready_pre_receive_hook(&repo_root)?;
    Ok(repo_root)
}

pub(crate) fn restore_git_pack_spans(
    state: &AppState,
    head: &GitHead,
    pack_spans: &[GitPackSpan],
    repo_root: &FsPath,
) -> Result<(), ApiError> {
    validate_git_pack_layout(pack_spans)
        .map_err(|error| ApiError::internal_message(error.to_string()))?;
    let final_span = pack_spans
        .last()
        .ok_or_else(|| ApiError::internal_message("Git head has no physical pack spans"))?;
    if final_span.last_sequence != head.push_sequence || final_span.head_oid != head.head_oid {
        return Err(ApiError::internal_message(
            "Git pack layout frontier does not match the logical head",
        ));
    }
    if repo_root.exists() {
        fs::remove_dir_all(repo_root).map_err(ApiError::internal)?;
    }
    run_git(
        None,
        &["init", "--bare", repo_root.to_string_lossy().as_ref()],
        "initializing Git snapshot repo",
    )?;
    for span in pack_spans {
        index_git_pack(state, repo_root, &span.object)?;
    }
    run_git(
        Some(repo_root),
        &[
            "update-ref",
            &format!("refs/heads/{DEFAULT_GIT_BRANCH}"),
            &head.head_oid,
        ],
        "restoring Git pack-layout head",
    )?;
    run_git(
        Some(repo_root),
        &["fsck", "--connectivity-only", &head.head_oid],
        "verifying restored Git pack layout",
    )?;
    run_git(
        Some(repo_root),
        &[
            "symbolic-ref",
            "HEAD",
            &format!("refs/heads/{DEFAULT_GIT_BRANCH}"),
        ],
        "setting restored Git snapshot head",
    )?;
    Ok(())
}

pub(crate) fn index_git_pack(
    state: &AppState,
    repo_root: &FsPath,
    pack: &SourceBlob,
) -> Result<(), ApiError> {
    let bytes = restore_object_bytes(state, pack, "pack")?;
    let size_bytes = bytes.len();
    let started_at = Instant::now();
    let output = crate::git::upload::git_process_output_with_timeout(
        Command::new("git")
            .arg("--git-dir")
            .arg(repo_root)
            .args(["index-pack", "--stdin"]),
        Some(bytes),
        state.runtime_budgets.git_command_timeout(),
    );
    let success = output.as_ref().is_ok_and(|output| output.status.success());
    let duration_ms = started_at.elapsed().as_millis();
    tracing::info!(
        operation = "index_pack",
        duration_ms,
        repo_git_index_pack_ms = duration_ms,
        size_bytes,
        success,
        "Git restore operation completed"
    );
    let output = output?;
    if !output.status.success() {
        return Err(ApiError::infrastructure_unavailable(format!(
            "restoring Git pack: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        )));
    }
    Ok(())
}

fn restore_object_bytes(
    state: &AppState,
    blob: &SourceBlob,
    object_kind: &'static str,
) -> Result<Vec<u8>, ApiError> {
    let started_at = Instant::now();
    let bytes = source_blob_bytes(state.object_store.as_ref(), blob).map_err(ApiError::from);
    let size_bytes = bytes.as_ref().map_or(blob.size_bytes, |bytes| {
        u64::try_from(bytes.len()).unwrap_or(u64::MAX)
    });
    let duration_ms = started_at.elapsed().as_millis();
    tracing::info!(
        operation = "object_retrieval",
        object_kind,
        duration_ms,
        repo_git_object_retrieval_ms = duration_ms,
        size_bytes,
        success = bytes.is_ok(),
        "Git restore operation completed"
    );
    bytes
}

pub(crate) fn install_first_push_pre_receive_hook(repo_root: &FsPath) -> Result<(), ApiError> {
    let hook = repo_root.join("hooks").join("pre-receive");
    let script = format!(
        "#!/bin/sh\ncount=0\nwhile read old new ref; do\n  count=$((count + 1))\n  if [ \"$ref\" != \"refs/heads/{DEFAULT_GIT_BRANCH}\" ]; then\n    echo \"Scope accepts pushes only to refs/heads/{DEFAULT_GIT_BRANCH}\" >&2\n    exit 1\n  fi\n  if [ \"$new\" = \"{EMPTY_GIT_OID}\" ]; then\n    echo \"Scope does not accept branch deletes in v0\" >&2\n    exit 1\n  fi\n  if [ \"$old\" != \"{EMPTY_GIT_OID}\" ]; then\n    echo \"Scope accepts only the initial branch push in v0\" >&2\n    exit 1\n  fi\ndone\nif [ \"$count\" -ne 1 ]; then\n  echo \"Scope accepts exactly one pushed branch in v0\" >&2\n  exit 1\nfi\n"
    );
    write_receive_pack_hook(&hook, &script)
}

pub(crate) fn install_ready_pre_receive_hook(repo_root: &FsPath) -> Result<(), ApiError> {
    let hook = repo_root.join("hooks").join("pre-receive");
    let script = format!(
        r#"#!/bin/sh
count=0
while read old new ref; do
  count=$((count + 1))
  if [ "$new" = "{EMPTY_GIT_OID}" ]; then
    echo "Scope does not accept branch deletes" >&2
    exit 1
  fi
  if [ "$ref" = "refs/heads/{DEFAULT_GIT_BRANCH}" ]; then
    if [ "$old" = "{EMPTY_GIT_OID}" ]; then
      echo "Scope accepts only updates to refs/heads/{DEFAULT_GIT_BRANCH}" >&2
      exit 1
    fi
    if ! git merge-base --is-ancestor "$old" "$new"; then
      echo "Scope rejects non-fast-forward pushes" >&2
      exit 1
    fi
    continue
  fi
  case "$ref" in
    refs/heads/*)
      if ! git cat-file -e "$new^{{commit}}"; then
        echo "Scope request refs must point at commits" >&2
        exit 1
      fi
      if [ "$old" != "{EMPTY_GIT_OID}" ] && ! git merge-base --is-ancestor "$old" "$new"; then
        echo "Scope rejects non-fast-forward request pushes" >&2
        exit 1
      fi
      ;;
    *)
      echo "Scope accepts pushes only to main or a named request branch" >&2
      exit 1
      ;;
  esac
done
if [ "$count" -ne 1 ]; then
  echo "Scope accepts exactly one pushed ref" >&2
  exit 1
fi
"#
    );
    write_receive_pack_hook(&hook, &script)
}

pub(crate) fn write_receive_pack_hook(hook: &FsPath, script: &str) -> Result<(), ApiError> {
    fs::write(hook, script).map_err(ApiError::internal)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut permissions = fs::metadata(hook)
            .map_err(ApiError::internal)?
            .permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(hook, permissions).map_err(ApiError::internal)?;
    }
    Ok(())
}

pub(crate) fn git_http_backend(
    staging_repo: &FsPath,
    method: &str,
    path_suffix: &str,
    query_string: &str,
    body: Vec<u8>,
    content_type: Option<String>,
    remote_user: &str,
) -> Result<CgiResponse, ApiError> {
    let staging_parent = staging_repo
        .parent()
        .ok_or_else(|| ApiError::internal_message("staging repo is missing a parent"))?;
    let repo_name = staging_repo
        .file_name()
        .and_then(|value| value.to_str())
        .ok_or_else(|| ApiError::internal_message("staging repo has invalid path"))?;
    let mut command = Command::new("git");
    command
        .arg("http-backend")
        .env("GIT_PROJECT_ROOT", staging_parent)
        .env("GIT_HTTP_EXPORT_ALL", "1")
        .env("REQUEST_METHOD", method)
        .env("PATH_INFO", format!("/{repo_name}/{path_suffix}"))
        .env("QUERY_STRING", query_string)
        .env("REMOTE_USER", remote_user)
        .env("CONTENT_LENGTH", body.len().to_string())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    if let Some(content_type) = content_type {
        command.env("CONTENT_TYPE", content_type);
    }

    let output = git_command_output_with_timeout(
        &mut command,
        Some(body),
        RuntimeBudgets::default_git_command_timeout(),
    )?;
    CgiResponse::parse(output)
}

#[allow(clippy::too_many_arguments)]
pub(crate) async fn git_http_backend_streaming(
    staging_repo: &FsPath,
    path_suffix: &str,
    body: Body,
    content_length: Option<u64>,
    max_bytes: usize,
    content_type: Option<String>,
    remote_user: &str,
) -> Result<CgiResponse, ApiError> {
    let receive_started = Instant::now();
    if content_length.is_some_and(|length| length > max_bytes as u64) {
        return Err(ApiError::payload_too_large(
            "git receive-pack body is too large",
        ));
    }
    let staging_parent = staging_repo
        .parent()
        .ok_or_else(|| ApiError::internal_message("staging repo is missing a parent"))?;
    let repo_name = staging_repo
        .file_name()
        .and_then(|value| value.to_str())
        .ok_or_else(|| ApiError::internal_message("staging repo has invalid path"))?;
    let mut command = tokio::process::Command::new("git");
    command
        .arg("http-backend")
        .env("GIT_PROJECT_ROOT", staging_parent)
        .env("GIT_HTTP_EXPORT_ALL", "1")
        .env("REQUEST_METHOD", "POST")
        .env("PATH_INFO", format!("/{repo_name}/{path_suffix}"))
        .env("QUERY_STRING", "")
        .env("REMOTE_USER", remote_user)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    scope_git_process::configure_process_group(command.as_std_mut());
    if let Some(content_length) = content_length {
        command.env("CONTENT_LENGTH", content_length.to_string());
    }
    if let Some(content_type) = content_type {
        command.env("CONTENT_TYPE", content_type);
    }

    let mut child = command.spawn().map_err(ApiError::internal)?;
    let mut process_group = GitProcessGroupGuard::new(child.id());
    let Some(mut stdin) = child.stdin.take() else {
        terminate_and_reap_git_child(&mut child, &mut process_group).await;
        return Err(ApiError::internal_message(
            "opening git http-backend stdin failed",
        ));
    };
    let Some(stdout) = child.stdout.take() else {
        terminate_and_reap_git_child(&mut child, &mut process_group).await;
        return Err(ApiError::internal_message(
            "opening git http-backend stdout failed",
        ));
    };
    let Some(stderr) = child.stderr.take() else {
        terminate_and_reap_git_child(&mut child, &mut process_group).await;
        return Err(ApiError::internal_message(
            "opening git http-backend stderr failed",
        ));
    };
    let mut stdout_task = tokio::spawn(read_git_pipe(stdout));
    let mut stderr_task = tokio::spawn(read_git_pipe(stderr));
    let process_timeout = RuntimeBudgets::default_git_command_timeout();
    let process_deadline = tokio::time::Instant::now() + process_timeout;
    let writer = async move {
        let mut stream = body.into_data_stream();
        let mut written = 0usize;
        loop {
            let next = tokio::time::timeout_at(process_deadline, stream.next())
                .await
                .map_err(|_| ApiError::infrastructure_unavailable("git request upload stalled"))?;
            let Some(chunk) = next else {
                break;
            };
            let chunk = chunk.map_err(ApiError::bad_request)?;
            written = written
                .checked_add(chunk.len())
                .ok_or_else(|| ApiError::payload_too_large("git receive-pack body is too large"))?;
            if written > max_bytes {
                return Err(ApiError::payload_too_large(
                    "git receive-pack body is too large",
                ));
            }
            stdin.write_all(&chunk).await.map_err(ApiError::internal)?;
        }
        stdin.shutdown().await.map_err(ApiError::internal)?;
        Ok::<usize, ApiError>(written)
    };
    let request_bytes = match tokio::time::timeout_at(process_deadline, writer).await {
        Ok(Ok(written)) => written,
        Ok(Err(error)) => {
            stop_git_http_backend(
                &mut child,
                &mut process_group,
                &mut stdout_task,
                &mut stderr_task,
            )
            .await;
            return Err(error);
        }
        Err(_) => {
            stop_git_http_backend(
                &mut child,
                &mut process_group,
                &mut stdout_task,
                &mut stderr_task,
            )
            .await;
            return Err(ApiError::infrastructure_unavailable(
                "git request upload timed out",
            ));
        }
    };
    let output = match tokio::time::timeout_at(
        process_deadline,
        collect_git_http_backend_output(&mut child, &mut stdout_task, &mut stderr_task),
    )
    .await
    {
        Ok(Ok(output)) => output,
        Ok(Err(error)) => {
            stop_git_http_backend(
                &mut child,
                &mut process_group,
                &mut stdout_task,
                &mut stderr_task,
            )
            .await;
            return Err(error);
        }
        Err(_) => {
            stop_git_http_backend(
                &mut child,
                &mut process_group,
                &mut stdout_task,
                &mut stderr_task,
            )
            .await;
            return Err(ApiError::infrastructure_unavailable(
                "git http-backend timed out",
            ));
        }
    };
    process_group.disarm();
    if !output.status.success() {
        return Err(ApiError::infrastructure_unavailable(format!(
            "git http-backend failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        )));
    }
    tracing::info!(
        request_bytes,
        receive_ms = receive_started.elapsed().as_millis(),
        "streamed Git receive-pack body"
    );
    CgiResponse::parse(output.stdout)
}

async fn read_git_pipe(mut pipe: impl tokio::io::AsyncRead + Unpin) -> std::io::Result<Vec<u8>> {
    let mut bytes = Vec::new();
    pipe.read_to_end(&mut bytes).await?;
    Ok(bytes)
}

async fn collect_git_http_backend_output(
    child: &mut tokio::process::Child,
    stdout_task: &mut JoinHandle<std::io::Result<Vec<u8>>>,
    stderr_task: &mut JoinHandle<std::io::Result<Vec<u8>>>,
) -> Result<std::process::Output, ApiError> {
    let stdout = join_git_pipe(stdout_task, "stdout").await?;
    let stderr = join_git_pipe(stderr_task, "stderr").await?;
    let status = child.wait().await.map_err(ApiError::internal)?;
    Ok(std::process::Output {
        status,
        stdout,
        stderr,
    })
}

async fn join_git_pipe(
    task: &mut JoinHandle<std::io::Result<Vec<u8>>>,
    pipe: &str,
) -> Result<Vec<u8>, ApiError> {
    task.await
        .map_err(|_| ApiError::internal_message(format!("git http-backend {pipe} task panicked")))?
        .map_err(ApiError::internal)
}

async fn stop_git_http_backend(
    child: &mut tokio::process::Child,
    process_group: &mut GitProcessGroupGuard,
    stdout_task: &mut JoinHandle<std::io::Result<Vec<u8>>>,
    stderr_task: &mut JoinHandle<std::io::Result<Vec<u8>>>,
) {
    terminate_and_reap_git_child(child, process_group).await;
    stdout_task.abort();
    stderr_task.abort();
    let _ = stdout_task.await;
    let _ = stderr_task.await;
}

async fn terminate_and_reap_git_child(
    child: &mut tokio::process::Child,
    process_group: &mut GitProcessGroupGuard,
) {
    process_group.kill();
    let _ = child.kill().await;
    let _ = child.wait().await;
}

struct GitProcessGroupGuard {
    process_id: Option<u32>,
}

impl GitProcessGroupGuard {
    fn new(process_id: Option<u32>) -> Self {
        Self { process_id }
    }

    fn kill(&mut self) {
        if let Some(process_id) = self.process_id.take() {
            scope_git_process::kill_process_group(process_id);
        }
    }

    fn disarm(&mut self) {
        self.process_id = None;
    }
}

impl Drop for GitProcessGroupGuard {
    fn drop(&mut self) {
        self.kill();
    }
}

pub(crate) struct CgiResponse {
    pub(crate) status: StatusCode,
    pub(crate) headers: Vec<(String, String)>,
    pub(crate) body: Vec<u8>,
}

impl CgiResponse {
    pub(crate) fn parse(output: Vec<u8>) -> Result<Self, ApiError> {
        let header_end = find_header_end(&output).ok_or_else(|| {
            ApiError::infrastructure_unavailable("git http-backend returned no headers")
        })?;
        let (headers, body) = output.split_at(header_end.0);
        let headers = String::from_utf8_lossy(headers);
        let mut status = StatusCode::OK;
        let mut parsed_headers = Vec::new();

        for line in headers
            .lines()
            .map(str::trim)
            .filter(|line| !line.is_empty())
        {
            let Some((name, value)) = line.split_once(':') else {
                continue;
            };
            if name.eq_ignore_ascii_case("Status") {
                let code = value
                    .split_whitespace()
                    .next()
                    .and_then(|code| code.parse::<u16>().ok())
                    .ok_or_else(|| {
                        ApiError::infrastructure_unavailable("invalid git CGI status")
                    })?;
                status = StatusCode::from_u16(code).map_err(ApiError::internal)?;
            } else {
                parsed_headers.push((name.trim().to_string(), value.trim().to_string()));
            }
        }

        Ok(Self {
            status,
            headers: parsed_headers,
            body: body[header_end.1..].to_vec(),
        })
    }

    pub(crate) fn into_response(self) -> Response {
        let mut builder = Response::builder().status(self.status);
        for (name, value) in self.headers {
            builder = builder.header(name, value);
        }
        builder
            .body(Body::from(self.body))
            .expect("git CGI response headers should be valid")
    }
}

pub(crate) fn find_header_end(bytes: &[u8]) -> Option<(usize, usize)> {
    bytes
        .windows(4)
        .position(|window| window == b"\r\n\r\n")
        .map(|index| (index, 4))
        .or_else(|| {
            bytes
                .windows(2)
                .position(|window| window == b"\n\n")
                .map(|index| (index, 2))
        })
}

#[cfg(all(test, target_os = "linux"))]
mod process_tests {
    use super::*;
    use tokio::io::{AsyncBufReadExt, BufReader};

    #[tokio::test]
    async fn terminating_git_child_reaps_child_and_kills_its_process_group() {
        let mut command = tokio::process::Command::new("sh");
        command
            .arg("-c")
            .arg("sleep 30 & printf '%s\\n' $!; wait")
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .kill_on_drop(true);
        scope_git_process::configure_process_group(command.as_std_mut());

        let mut child = command.spawn().expect("spawn test process group");
        let stdout = child.stdout.take().expect("test child stdout");
        let mut stdout = BufReader::new(stdout);
        let mut descendant = String::new();
        stdout
            .read_line(&mut descendant)
            .await
            .expect("read descendant pid");
        let descendant = descendant.trim().parse::<u32>().expect("descendant pid");

        let mut process_group = GitProcessGroupGuard::new(child.id());
        terminate_and_reap_git_child(&mut child, &mut process_group).await;

        assert!(child.id().is_none(), "direct child was not reaped");
        let mut descendant_state = None;
        for _ in 0..100 {
            descendant_state = fs::read_to_string(format!("/proc/{descendant}/stat"))
                .ok()
                .and_then(|stat| {
                    let command_end = stat.rfind(')')?;
                    stat[command_end + 2..]
                        .split_whitespace()
                        .next()
                        .map(str::to_string)
                });
            if descendant_state.as_deref().is_none_or(|state| state == "Z") {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
        assert!(
            descendant_state.as_deref().is_none_or(|state| state == "Z"),
            "descendant survived process-group kill in state {descendant_state:?}"
        );
    }
}
