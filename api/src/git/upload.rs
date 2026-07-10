use crate::domain::policy::Principal;
use crate::domain::projection::{Projection, ProjectionViewKey, project_graph};
use crate::domain::store::{RepoPublicationState, RepositoryActor, is_supported_git_file_mode};
use crate::{
    auth::scope::principal_for_user_id,
    config::{DEFAULT_GIT_BRANCH, GIT_UPLOAD_PACK, UNPUBLISHED_GIT_ERROR},
    error::ApiError,
    git::{GitRemoteMode, git_read_scope_user, storage::cached_raw_git_snapshot_repo},
    object_store::source_blob_bytes,
    runtime_budgets::{RuntimeBudgets, RuntimePermit},
    state::AppState,
    state::{ensure_repo_read, find_repo},
};
use axum::{
    body::Body,
    http::{
        HeaderMap, StatusCode,
        header::{CACHE_CONTROL, CONTENT_TYPE},
    },
    response::{IntoResponse, Response},
};
use sha1::{Digest, Sha1};
use std::{
    collections::BTreeMap,
    fs,
    io::{ErrorKind, Read, Write},
    path::{Path as FsPath, PathBuf},
    process::{Child, Output},
    process::{Command, Stdio},
    sync::atomic::{AtomicU64, Ordering},
    thread,
    time::{Duration, Instant},
};
const PROJECTION_CACHE_SEMANTICS_VERSION: &str = "shared-projection-view-v1";
const GIT_STDERR_DIAGNOSTIC_BYTES: usize = 8 * 1024;
static GIT_CACHE_ATTEMPT: AtomicU64 = AtomicU64::new(1);

pub(crate) async fn git_projection_for_request(
    state: &AppState,
    headers: &HeaderMap,
    owner: &str,
    repo_name: &str,
    mode: GitRemoteMode,
) -> Result<Projection, ApiError> {
    let (repo, principal) =
        git_read_principal_for_request(state, headers, owner, repo_name, mode).await?;
    if repo.record.publication_state != RepoPublicationState::Published {
        return unpublished_git_read_error(&repo, owner, repo_name, &principal);
    }

    ensure_repo_read(state, &repo, &principal)?;
    let access = repo.access_for_principal(&principal);
    if mode == GitRemoteMode::Permissioned && access.actor == RepositoryActor::Public {
        return Err(ApiError::forbidden("repo membership required"));
    }
    let view_key = ProjectionViewKey::from_access(access);
    Ok(project_graph(
        &repo.policy,
        &repo.graph,
        &repo.visibility_events,
        view_key,
    ))
}

pub(crate) async fn git_upload_pack_repo_for_request(
    state: &AppState,
    headers: &HeaderMap,
    owner: &str,
    repo_name: &str,
    mode: GitRemoteMode,
) -> Result<PathBuf, ApiError> {
    if mode == GitRemoteMode::Permissioned
        && let Some(repo_path) =
            private_live_repo_for_request(state, headers, owner, repo_name, mode).await?
    {
        return Ok(repo_path);
    }

    let projection = match git_projection_for_request(state, headers, owner, repo_name, mode).await
    {
        Ok(projection) => projection,
        Err(error) if mode == GitRemoteMode::Public && error.status() == StatusCode::NOT_FOUND => {
            return Err(git_upload_pack_auth_required());
        }
        Err(error) => return Err(error),
    };
    projection_bare_repo_for_state(state, &projection)
}

pub(crate) fn git_upload_pack_auth_required() -> ApiError {
    ApiError::unauthorized("Git credentials required")
}

pub(crate) async fn private_live_repo_for_request(
    state: &AppState,
    headers: &HeaderMap,
    owner: &str,
    repo_name: &str,
    mode: GitRemoteMode,
) -> Result<Option<PathBuf>, ApiError> {
    let (repo, principal) =
        git_read_principal_for_request(state, headers, owner, repo_name, mode).await?;
    if repo.record.publication_state != RepoPublicationState::Published {
        if repo.access_for_principal(&principal).actor == RepositoryActor::Owner {
            return Err(ApiError::forbidden(UNPUBLISHED_GIT_ERROR));
        }
        return Ok(None);
    }
    ensure_repo_read(state, &repo, &principal)?;
    let access = repo.access_for_principal(&principal);
    if ProjectionViewKey::from_access(access) != ProjectionViewKey::Private {
        return Ok(None);
    }
    let Some(snapshot) = repo.git_snapshot.as_ref() else {
        return Ok(None);
    };
    cached_raw_git_snapshot_repo(state, snapshot).map(Some)
}

async fn git_read_principal_for_request(
    state: &AppState,
    headers: &HeaderMap,
    owner: &str,
    repo_name: &str,
    mode: GitRemoteMode,
) -> Result<(crate::domain::store::StoredRepository, Principal), ApiError> {
    match mode {
        GitRemoteMode::Public => {
            let repo = find_repo(state, owner, repo_name).await?;
            Ok((repo, Principal::public()))
        }
        GitRemoteMode::Permissioned => {
            let user = git_read_scope_user(state, headers).await?;
            let repo = find_repo(state, owner, repo_name).await?;
            let principal = principal_for_user_id(&repo, &user.id);
            Ok((repo, principal))
        }
    }
}

fn unpublished_git_read_error(
    repo: &crate::domain::store::StoredRepository,
    owner: &str,
    repo_name: &str,
    principal: &Principal,
) -> Result<Projection, ApiError> {
    if repo.access_for_principal(principal).actor == RepositoryActor::Owner {
        Err(ApiError::forbidden(UNPUBLISHED_GIT_ERROR))
    } else {
        Err(ApiError::not_found(format!(
            "repo {owner}/{repo_name} not found"
        )))
    }
}

pub(crate) fn projection_bare_repo(
    store: &dyn crate::object_store::ObjectStore,
    cache_root: &FsPath,
    projection: &Projection,
) -> Result<PathBuf, ApiError> {
    let cache_key = projection_cache_key(projection);
    let repo_path = cache_root.join(format!("{cache_key}.git"));
    if repo_path
        .join("refs")
        .join("heads")
        .join(DEFAULT_GIT_BRANCH)
        .is_file()
    {
        return Ok(repo_path);
    }

    let attempt = GIT_CACHE_ATTEMPT.fetch_add(1, Ordering::Relaxed);
    let temp_path = cache_root.join(format!(
        "{cache_key}.{}.{}.tmp",
        std::process::id(),
        attempt
    ));
    if temp_path.exists() {
        fs::remove_dir_all(&temp_path).map_err(ApiError::internal)?;
    }

    git_command_output(
        Command::new("git")
            .arg("init")
            .arg("--bare")
            .arg(&temp_path),
        None,
    )?;
    git_command_output(
        Command::new("git")
            .arg("--git-dir")
            .arg(&temp_path)
            .arg("symbolic-ref")
            .arg("HEAD")
            .arg(format!("refs/heads/{DEFAULT_GIT_BRANCH}")),
        None,
    )?;

    let index_path = cache_root.join(format!(
        "{cache_key}.{}.{}.index",
        std::process::id(),
        attempt
    ));
    if index_path.exists() {
        fs::remove_file(&index_path).map_err(ApiError::internal)?;
    }

    let mut visible_tree = BTreeMap::new();
    let mut parent_commit: Option<String> = None;
    if projection.commits.is_empty() {
        let tree = write_projection_tree(&temp_path, &index_path, &visible_tree)?;
        parent_commit = Some(git_commit_tree(
            &temp_path,
            &tree,
            None,
            "Empty Scope projection\n",
        )?);
    }

    for projected in &projection.commits {
        for change in &projected.changes {
            let path = change.path.as_str().to_string();
            match &change.new_content {
                Some(blob) => {
                    visible_tree.insert(
                        path,
                        ProjectionTreeFile {
                            bytes: source_blob_bytes(store, blob)?,
                            git_file_mode: blob.git_file_mode.clone(),
                        },
                    );
                }
                None => {
                    visible_tree.remove(&path);
                }
            }
        }
        let tree = write_projection_tree(&temp_path, &index_path, &visible_tree)?;
        let message = format!("{}\n", projected.message);
        parent_commit = Some(git_commit_tree(
            &temp_path,
            &tree,
            parent_commit.as_deref(),
            &message,
        )?);
    }

    let commit = parent_commit.ok_or_else(|| ApiError::internal_message("missing Git commit"))?;
    git_command_output(
        Command::new("git")
            .arg("--git-dir")
            .arg(&temp_path)
            .arg("update-ref")
            .arg(format!("refs/heads/{DEFAULT_GIT_BRANCH}"))
            .arg(commit.trim()),
        None,
    )?;

    let _ = fs::remove_file(&index_path);
    match fs::rename(&temp_path, &repo_path) {
        Ok(()) => Ok(repo_path),
        Err(error) if repo_path.exists() => {
            let _ = fs::remove_dir_all(&temp_path);
            tracing::debug!(%error, path = %repo_path.display(), "using concurrently-created Git projection cache");
            Ok(repo_path)
        }
        Err(error) => Err(ApiError::internal(error)),
    }
}

pub(crate) fn projection_bare_repo_for_state(
    state: &AppState,
    projection: &Projection,
) -> Result<PathBuf, ApiError> {
    let cache_root = state.git_cache_root()?;
    let cache_key = projection_cache_key(projection);
    let repo_path = cache_root.join(format!("{cache_key}.git"));
    if repo_path
        .join("refs")
        .join("heads")
        .join(DEFAULT_GIT_BRANCH)
        .is_file()
    {
        return Ok(repo_path);
    }

    let _permit = state.runtime_budgets.try_projection_build()?;
    projection_bare_repo(state.object_store.as_ref(), &cache_root, projection)
}

pub(crate) fn projection_cache_key(projection: &Projection) -> String {
    let mut hasher = Sha1::new();
    hash_field(
        &mut hasher,
        b"semantics",
        PROJECTION_CACHE_SEMANTICS_VERSION.as_bytes(),
    );
    hash_field(&mut hasher, b"repo", projection.repo_id.as_bytes());
    hash_field(
        &mut hasher,
        b"view",
        projection.view_key.as_str().as_bytes(),
    );
    for commit in &projection.commits {
        hash_field(&mut hasher, b"commit", commit.projected_id.as_bytes());
        hash_field(&mut hasher, b"logical", commit.logical_commit_id.as_bytes());
        if let Some(parent) = &commit.parent_projected_id {
            hash_field(&mut hasher, b"parent", parent.as_bytes());
        }
        hash_field(&mut hasher, b"message", commit.message.as_bytes());
        for change in &commit.changes {
            hash_field(&mut hasher, b"path", change.path.as_str().as_bytes());
            match &change.new_content {
                Some(blob) => {
                    hash_field(&mut hasher, b"sha256", blob.sha256.as_bytes());
                    hash_field(&mut hasher, b"git_oid", blob.git_oid.as_bytes());
                    hash_field(&mut hasher, b"mode", blob.git_file_mode.as_bytes());
                    hash_field(&mut hasher, b"size", blob.size_bytes.to_string().as_bytes());
                }
                None => hash_field(&mut hasher, b"delete", b""),
            }
        }
    }
    hex::encode(hasher.finalize())
}

pub(crate) fn hash_field(hasher: &mut Sha1, label: &[u8], value: &[u8]) {
    hasher.update((label.len() as u64).to_be_bytes());
    hasher.update(label);
    hasher.update((value.len() as u64).to_be_bytes());
    hasher.update(value);
}

pub(crate) fn write_projection_tree(
    repo_path: &FsPath,
    index_path: &FsPath,
    visible_tree: &BTreeMap<String, ProjectionTreeFile>,
) -> Result<String, ApiError> {
    if index_path.exists() {
        fs::remove_file(index_path).map_err(ApiError::internal)?;
    }
    git_index_command(
        Command::new("git")
            .arg("--git-dir")
            .arg(repo_path)
            .arg("read-tree")
            .arg("--empty"),
        index_path,
        None,
    )?;

    let mut index_info = Vec::new();
    for (path, file) in visible_tree {
        if !is_supported_git_file_mode(&file.git_file_mode) {
            return Err(ApiError::internal_message(format!(
                "projected Git path {path} has unsupported mode {}",
                file.git_file_mode
            )));
        }
        let oid = git_command_output(
            Command::new("git")
                .arg("--git-dir")
                .arg(repo_path)
                .arg("hash-object")
                .arg("-w")
                .arg("--stdin"),
            Some(&file.bytes),
        )?;
        let oid = String::from_utf8(oid).map_err(ApiError::bad_request)?;
        let relative_path = git_relative_path(path)?;
        index_info.extend_from_slice(
            format!(
                "{} blob {}\t{relative_path}\n",
                file.git_file_mode,
                oid.trim()
            )
            .as_bytes(),
        );
    }

    if !index_info.is_empty() {
        git_index_command(
            Command::new("git")
                .arg("--git-dir")
                .arg(repo_path)
                .arg("update-index")
                .arg("--index-info"),
            index_path,
            Some(&index_info),
        )?;
    }
    let tree = git_index_command(
        Command::new("git")
            .arg("--git-dir")
            .arg(repo_path)
            .arg("write-tree"),
        index_path,
        None,
    )?;
    let tree = String::from_utf8(tree).map_err(ApiError::bad_request)?;
    Ok(tree.trim().to_string())
}

pub(crate) struct ProjectionTreeFile {
    pub(crate) bytes: Vec<u8>,
    pub(crate) git_file_mode: String,
}

pub(crate) fn git_relative_path(path: &str) -> Result<String, ApiError> {
    let Some(relative) = path.strip_prefix('/') else {
        return Err(ApiError::internal_message(format!(
            "projected Git path {path} is not absolute"
        )));
    };
    if relative.is_empty()
        || relative == "."
        || relative == ".."
        || relative.starts_with("../")
        || relative.contains("/../")
        || relative.contains('\\')
        || relative.as_bytes().contains(&0)
    {
        return Err(ApiError::internal_message(format!(
            "projected Git path {path} cannot be served"
        )));
    }
    Ok(relative.to_string())
}

pub(crate) fn git_commit_tree(
    repo_path: &FsPath,
    tree: &str,
    parent: Option<&str>,
    message: &str,
) -> Result<String, ApiError> {
    let mut command = Command::new("git");
    command
        .arg("--git-dir")
        .arg(repo_path)
        .arg("commit-tree")
        .arg(tree)
        .env("GIT_AUTHOR_NAME", "Scope")
        .env("GIT_AUTHOR_EMAIL", "scope@example.invalid")
        .env("GIT_AUTHOR_DATE", "2000-01-01T00:00:00Z")
        .env("GIT_COMMITTER_NAME", "Scope")
        .env("GIT_COMMITTER_EMAIL", "scope@example.invalid")
        .env("GIT_COMMITTER_DATE", "2000-01-01T00:00:00Z");
    if let Some(parent) = parent {
        command.arg("-p").arg(parent.trim());
    }
    let output = git_command_output(&mut command, Some(message.as_bytes()))?;
    String::from_utf8(output).map_err(ApiError::bad_request)
}

pub(crate) fn git_index_command(
    command: &mut Command,
    index_path: &FsPath,
    stdin: Option<&[u8]>,
) -> Result<Vec<u8>, ApiError> {
    command.env("GIT_INDEX_FILE", index_path);
    git_command_output(command, stdin)
}

pub(crate) fn git_command_output(
    command: &mut Command,
    stdin: Option<&[u8]>,
) -> Result<Vec<u8>, ApiError> {
    git_command_output_with_timeout(
        command,
        stdin.map(Vec::from),
        RuntimeBudgets::default_git_command_timeout(),
    )
}

pub(crate) fn git_command_output_with_timeout(
    command: &mut Command,
    stdin: Option<Vec<u8>>,
    timeout: Duration,
) -> Result<Vec<u8>, ApiError> {
    let output = git_process_output_with_timeout(command, stdin, timeout)?;
    if output.status.success() {
        return Ok(output.stdout);
    }

    let stderr = truncated_git_stderr(&output.stderr);
    Err(ApiError::service_unavailable(stderr.trim()))
}

pub(crate) fn git_process_output_with_timeout(
    command: &mut Command,
    stdin: Option<Vec<u8>>,
    timeout: Duration,
) -> Result<Output, ApiError> {
    if stdin.is_some() {
        command.stdin(Stdio::piped());
    }
    configure_process_group(command);
    let mut child = command
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| ApiError::service_unavailable(format!("failed to run Git: {error}")))?;
    let stdin_writer = if let Some(input) = stdin {
        let mut child_stdin = child
            .stdin
            .take()
            .ok_or_else(|| ApiError::internal_message("failed to open Git stdin"))?;
        Some(thread::spawn(move || {
            child_stdin.write_all(&input)?;
            child_stdin.flush()
        }))
    } else {
        None
    };
    wait_with_timeout(child, stdin_writer, timeout, "Git command")
}

fn wait_with_timeout(
    mut child: std::process::Child,
    stdin_writer: Option<thread::JoinHandle<std::io::Result<()>>>,
    timeout: Duration,
    action: &str,
) -> Result<Output, ApiError> {
    let mut stdout = child
        .stdout
        .take()
        .ok_or_else(|| ApiError::internal_message("failed to open Git stdout"))?;
    let stderr = child
        .stderr
        .take()
        .ok_or_else(|| ApiError::internal_message("failed to open Git stderr"))?;
    let stdout_reader = thread::spawn(move || {
        let mut bytes = Vec::new();
        stdout.read_to_end(&mut bytes).map(|_| bytes)
    });
    let stderr_reader = thread::spawn(move || read_stderr_diagnostic(stderr));

    let started_at = Instant::now();
    let mut status = None;
    let status = loop {
        if status.is_none() {
            status = child.try_wait().map_err(ApiError::internal)?;
        }
        let stdin_done = stdin_writer
            .as_ref()
            .is_none_or(thread::JoinHandle::is_finished);
        if stdout_reader.is_finished()
            && stderr_reader.is_finished()
            && stdin_done
            && let Some(status) = status.take()
        {
            break status;
        }
        if started_at.elapsed() >= timeout {
            kill_process_group(&mut child);
            if status.is_none() {
                let _status = child.wait().map_err(ApiError::internal)?;
            }
            drop(stdin_writer);
            let _stdout = join_reader(stdout_reader)?;
            let stderr = join_reader(stderr_reader)?;
            let message = truncated_git_stderr(&stderr);
            return Err(ApiError::service_unavailable(format!(
                "{action} timed out after {}ms{}{}",
                timeout.as_millis(),
                if message.is_empty() { "" } else { ": " },
                message
            )));
        }
        let remaining = timeout.saturating_sub(started_at.elapsed());
        thread::sleep(remaining.min(Duration::from_millis(1)));
    };

    if let Some(stdin_writer) = stdin_writer {
        join_writer(stdin_writer)?;
    }
    let stdout = join_reader(stdout_reader)?;
    let stderr = join_reader(stderr_reader)?;
    Ok(Output {
        status,
        stdout,
        stderr,
    })
}

fn join_reader(handle: thread::JoinHandle<std::io::Result<Vec<u8>>>) -> Result<Vec<u8>, ApiError> {
    handle
        .join()
        .map_err(|_| ApiError::internal_message("Git output reader panicked"))?
        .map_err(ApiError::internal)
}

fn join_writer(handle: thread::JoinHandle<std::io::Result<()>>) -> Result<(), ApiError> {
    match handle
        .join()
        .map_err(|_| ApiError::internal_message("Git stdin writer panicked"))?
    {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == ErrorKind::BrokenPipe => Ok(()),
        Err(error) => Err(ApiError::internal(error)),
    }
}

fn read_stderr_diagnostic(mut stderr: impl Read) -> std::io::Result<Vec<u8>> {
    let max_retained = GIT_STDERR_DIAGNOSTIC_BYTES.saturating_add(1);
    let mut retained = Vec::new();
    let mut buffer = [0_u8; 8 * 1024];
    loop {
        let read = stderr.read(&mut buffer)?;
        if read == 0 {
            return Ok(retained);
        }
        let remaining = max_retained.saturating_sub(retained.len());
        if remaining > 0 {
            retained.extend_from_slice(&buffer[..read.min(remaining)]);
        }
    }
}

#[cfg(unix)]
fn configure_process_group(command: &mut Command) {
    use std::os::unix::process::CommandExt;
    command.process_group(0);
}

#[cfg(not(unix))]
fn configure_process_group(_command: &mut Command) {}

#[cfg(unix)]
fn kill_process_group(child: &mut Child) {
    let group = format!("-{}", child.id());
    let _ = Command::new("kill")
        .arg("-KILL")
        .arg("--")
        .arg(group)
        .status();
    let _ = child.kill();
}

#[cfg(not(unix))]
fn kill_process_group(child: &mut Child) {
    // Scope's API runs on Unix. Non-Unix builds only kill the direct child here;
    // descendants can hold output pipes open until they exit.
    let _ = child.kill();
}

fn truncated_git_stderr(stderr: &[u8]) -> String {
    let mut message = String::from_utf8_lossy(stderr).trim().to_string();
    if message.len() > GIT_STDERR_DIAGNOSTIC_BYTES {
        let mut end = 0;
        for (index, character) in message.char_indices() {
            let next = index + character.len_utf8();
            if next > GIT_STDERR_DIAGNOSTIC_BYTES {
                break;
            }
            end = next;
        }
        message.truncate(end);
        message.push_str("...");
    }
    message
}

pub(crate) async fn git_upload_pack_response(
    repo_path: &FsPath,
    request: &[u8],
    timeout: Duration,
    permit: RuntimePermit,
) -> Result<Response, ApiError> {
    let repo_path = repo_path.to_path_buf();
    let request = request.to_vec();
    let output = tokio::task::spawn_blocking(move || {
        let _permit = permit;
        let mut command = Command::new("git");
        command
            .arg("upload-pack")
            .arg("--stateless-rpc")
            .arg(repo_path);
        git_process_output_with_timeout(&mut command, Some(request), timeout)
    })
    .await
    .map_err(ApiError::internal)??;
    if !output.status.success() {
        return Err(ApiError::service_unavailable(format!(
            "git upload-pack failed: {}",
            truncated_git_stderr(&output.stderr)
        )));
    }

    Ok(git_response(
        "application/x-git-upload-pack-result",
        output.stdout,
    ))
}

pub(crate) fn git_upload_pack_advertisement(repo_path: &FsPath, timeout: Duration) -> Response {
    match git_command_output_with_timeout(
        Command::new("git")
            .arg("upload-pack")
            .arg("--stateless-rpc")
            .arg("--advertise-refs")
            .arg(repo_path),
        None,
        timeout,
    ) {
        Ok(advertisement) => {
            let mut body = pkt_line(format!("# service={GIT_UPLOAD_PACK}\n").as_bytes());
            body.extend_from_slice(b"0000");
            body.extend(advertisement);
            git_response("application/x-git-upload-pack-advertisement", body)
        }
        Err(error) => git_advertisement_error(error.into_message()),
    }
}

pub(crate) fn git_response(content_type: &'static str, body: Vec<u8>) -> Response {
    (
        StatusCode::OK,
        [(CONTENT_TYPE, content_type), (CACHE_CONTROL, "no-cache")],
        Body::from(body),
    )
        .into_response()
}

pub(crate) fn git_advertisement_error(message: impl AsRef<str>) -> Response {
    git_response(
        "application/x-git-upload-pack-advertisement",
        git_error_body(message.as_ref()),
    )
}

pub(crate) fn git_upload_pack_error(message: impl AsRef<str>) -> Response {
    git_response(
        "application/x-git-upload-pack-result",
        git_error_body(message.as_ref()),
    )
}

pub(crate) fn git_error_body(message: &str) -> Vec<u8> {
    pkt_line(format!("ERR {message}\n").as_bytes())
}

pub(crate) fn pkt_line(payload: &[u8]) -> Vec<u8> {
    let len = payload.len() + 4;
    let mut line = format!("{len:04x}").into_bytes();
    line.extend_from_slice(payload);
    line
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stderr_truncation_preserves_utf8_boundaries() {
        let stderr = "é".repeat(GIT_STDERR_DIAGNOSTIC_BYTES);

        let truncated = truncated_git_stderr(stderr.as_bytes());

        assert!(truncated.ends_with("..."));
        assert!(truncated.is_char_boundary(truncated.len() - 3));
    }

    #[cfg(unix)]
    #[test]
    fn git_timeout_kills_descendants_that_hold_output_pipes() {
        let mut command = Command::new("sh");
        command.arg("-c").arg("(sleep 5) & sleep 5");
        let started_at = Instant::now();

        let error = git_command_output_with_timeout(&mut command, None, Duration::from_millis(25))
            .unwrap_err();

        assert_eq!(error.status(), StatusCode::SERVICE_UNAVAILABLE);
        assert!(error.message().contains("timed out"));
        assert!(started_at.elapsed() < Duration::from_secs(2));
    }
}
