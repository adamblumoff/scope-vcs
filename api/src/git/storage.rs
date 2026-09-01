use crate::{
    config::{DEFAULT_GIT_BRANCH, EMPTY_GIT_OID, RECEIVE_PACK_STAGING_BYTES},
    error::ApiError,
    git::import::run_git,
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
use scope_domain::{
    projection::{ProjectionViewKey, project_graph},
    repository::{RepoLifecycleState, RepositoryIncarnation},
};
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
    incarnation: &RepositoryIncarnation,
) -> Result<PathBuf, ApiError> {
    let mut bytes = [0_u8; RECEIVE_PACK_STAGING_BYTES];
    getrandom::fill(&mut bytes).map_err(|error| {
        ApiError::internal_message(format!(
            "failed to create receive-pack staging path: {error}"
        ))
    })?;
    let base_dir = state.data_dir.as_ref().clone();
    let digest = repository_storage_key(incarnation);
    ensure_private_dir(&base_dir)?;
    Ok(base_dir
        .join("git-rx")
        .join(format!("{digest}-{}.git", hex::encode(bytes))))
}

pub(crate) fn receive_pack_staging_repo_prefix(incarnation: &RepositoryIncarnation) -> String {
    repository_storage_key(incarnation)
}

pub(crate) fn request_ref_store_repo_path(
    state: &AppState,
    incarnation: &RepositoryIncarnation,
) -> PathBuf {
    git_repo_storage_root(state)
        .join("git-request-refs")
        .join(format!("{}.git", repository_storage_key(incarnation)))
}

pub(crate) fn repository_storage_key(incarnation: &RepositoryIncarnation) -> String {
    let mut hasher = Sha256::new();
    for value in [
        incarnation.repository_id().as_bytes(),
        incarnation.incarnation_id().as_bytes(),
    ] {
        hasher.update((value.len() as u64).to_be_bytes());
        hasher.update(value);
    }
    hex::encode(&hasher.finalize()[..16])
}

pub(crate) fn git_repo_storage_root(state: &AppState) -> PathBuf {
    state.data_dir.as_ref().clone()
}

pub(crate) fn delete_repo_storage(
    state: &AppState,
    cleanup: &scope_domain::repo_actions::RepoStorageCleanup,
) -> Result<(), ApiError> {
    if !state
        .repository_engine
        .delete_repository_cache(&cleanup.incarnation)?
    {
        return Err(ApiError::infrastructure_unavailable(
            "repository Git cache is still in use",
        ));
    }
    remove_dir_if_exists(&request_ref_store_repo_path(state, &cleanup.incarnation))?;

    let rx_root = git_repo_storage_root(state).join("git-rx");
    let prefix = receive_pack_staging_repo_prefix(&cleanup.incarnation);
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

pub(crate) fn remove_dir_if_exists(path: &FsPath) -> Result<(), ApiError> {
    match fs::remove_dir_all(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(ApiError::internal(error)),
    }
}

pub(crate) fn ensure_first_push_receive_pack_staging_repo(
    state: &AppState,
    incarnation: &RepositoryIncarnation,
) -> Result<PathBuf, ApiError> {
    let repo_root = receive_pack_staging_repo_path(state, incarnation)?;
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
    incarnation: &RepositoryIncarnation,
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
    if repo.incarnation != *incarnation {
        return Err(ApiError::conflict(
            "repository was recreated during push preparation",
        ));
    }
    let repo_root = receive_pack_staging_repo_path(state, incarnation)?;
    if let Some(parent) = repo_root.parent() {
        ensure_private_dir(parent)?;
    }
    if let Some(head) = repo.git_head.as_ref() {
        let seed_repo = state.repository_engine.materialize_repository(
            state,
            incarnation,
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
            incarnation,
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
