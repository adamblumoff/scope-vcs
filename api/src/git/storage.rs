use crate::domain::policy::Principal;
use crate::domain::projection::{ProjectionViewKey, project_graph};
use crate::domain::store::{RepoPublicationState, SourceBlob};
use crate::{
    config::{
        DEFAULT_GIT_BRANCH, EMPTY_GIT_OID, MAX_GIT_SEGMENT_CHAIN_DEPTH, RECEIVE_PACK_STAGING_BYTES,
    },
    error::ApiError,
    git::import::{run_git, safe_repo_key},
    git::upload::{git_command_output_with_timeout, projection_bare_repo_for_state},
    object_store::source_blob_bytes,
    persistence::ensure_private_dir,
    runtime_budgets::RuntimeBudgets,
    state::{AppState, find_repo},
};
use axum::{body::Body, http::StatusCode, response::Response};
use futures_util::StreamExt;
use scope_core::git_segments::{GitSegmentManifest, is_git_segment_manifest};
use sha2::{Digest, Sha256};
use std::time::Instant;
use std::{
    fs,
    path::{Path as FsPath, PathBuf},
    process::{Command, Stdio},
    sync::atomic::{AtomicU64, Ordering},
};
use tokio::io::AsyncWriteExt;

static RAW_GIT_CACHE_ATTEMPT: AtomicU64 = AtomicU64::new(1);

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
    let repo_id = crate::domain::store::repo_id(owner, repo_name);
    let digest = Sha256::digest(repo_id.as_bytes());
    let digest = hex::encode(digest);
    ensure_private_dir(&base_dir)?;
    Ok(base_dir
        .join("git-rx")
        .join(format!("{}-{}.git", &digest[..16], hex::encode(bytes))))
}

pub(crate) fn receive_pack_staging_repo_prefix(owner: &str, repo_name: &str) -> String {
    let repo_id = crate::domain::store::repo_id(owner, repo_name);
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

pub(crate) fn raw_git_cache_key(manifest: &SourceBlob) -> String {
    manifest
        .sha256
        .get(..16)
        .unwrap_or(manifest.sha256.as_str())
        .to_string()
}

pub(crate) fn raw_git_cache_path(
    state: &AppState,
    snapshot: &SourceBlob,
) -> Result<PathBuf, ApiError> {
    Ok(state.raw_git_cache.path_for(snapshot))
}

pub(crate) fn cached_raw_git_repo(
    state: &AppState,
    snapshot: &SourceBlob,
) -> Result<crate::git::cache::GitRepoHandle, ApiError> {
    let repo = state.raw_git_cache.lease(snapshot)?;
    let repo_path = repo.as_ref().to_path_buf();
    if repo_path
        .join("refs")
        .join("heads")
        .join(DEFAULT_GIT_BRANCH)
        .is_file()
    {
        return Ok(repo);
    }

    let cache_root = state.git_cache_root()?;
    let cache_key = raw_git_cache_key(snapshot);
    let attempt = RAW_GIT_CACHE_ATTEMPT.fetch_add(1, Ordering::Relaxed);
    let temp_path = cache_root.join(format!(
        "raw-{cache_key}.{}.{}.tmp",
        std::process::id(),
        attempt
    ));
    let _permit = state.runtime_budgets.try_projection_build()?;
    if let Err(error) = restore_git_segments(state, snapshot, &temp_path) {
        let _ = fs::remove_dir_all(&temp_path);
        return Err(error);
    }
    match fs::rename(&temp_path, &repo_path) {
        Ok(()) => {
            state.raw_git_cache.note_materialized(&repo_path)?;
            Ok(repo)
        }
        Err(error) if repo_path.exists() => {
            let _ = fs::remove_dir_all(&temp_path);
            tracing::debug!(%error, path = %repo_path.display(), "using concurrently-created raw Git snapshot cache");
            state.raw_git_cache.note_materialized(&repo_path)?;
            Ok(repo)
        }
        Err(error) => {
            let _ = fs::remove_dir_all(&temp_path);
            Err(ApiError::internal(error))
        }
    }
}

pub(crate) fn delete_repo_storage(
    state: &AppState,
    owner: &str,
    repo_name: &str,
) -> Result<(), ApiError> {
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

pub(crate) async fn ensure_published_receive_pack_staging_repo(
    state: &AppState,
    owner: &str,
    repo_name: &str,
    author_id: &str,
) -> Result<PathBuf, ApiError> {
    let repo = state
        .metadata
        .git_push_context(owner, repo_name, author_id)
        .await?
        .ok_or_else(|| ApiError::not_found(format!("repo {owner}/{repo_name} not found")))?;
    if repo.publication_state != RepoPublicationState::Published {
        return Err(ApiError::conflict("repo must be published before push"));
    }
    let repo_root = receive_pack_staging_repo_path(state, owner, repo_name)?;
    if let Some(parent) = repo_root.parent() {
        ensure_private_dir(parent)?;
    }
    if let Some(head) = repo.git_head.as_ref() {
        let seed_repo = cached_raw_git_repo(state, &head.manifest)?;
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
            kind: crate::domain::policy::PrincipalKind::User,
        };
        let view_key = ProjectionViewKey::from_access(repo.access_for_principal(&principal));
        let projection =
            project_graph(&repo.policy, &repo.graph, &repo.visibility_events, view_key);
        let seed_repo = projection_bare_repo_for_state(state, &projection)?;
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
    install_published_pre_receive_hook(&repo_root)?;
    Ok(repo_root)
}

pub(crate) fn restore_git_segments(
    state: &AppState,
    snapshot: &crate::domain::store::SourceBlob,
    repo_root: &FsPath,
) -> Result<(), ApiError> {
    if repo_root.exists() {
        fs::remove_dir_all(repo_root).map_err(ApiError::internal)?;
    }
    run_git(
        None,
        &["init", "--bare", repo_root.to_string_lossy().as_ref()],
        "initializing Git snapshot repo",
    )?;
    let mut segments = Vec::new();
    let mut restored_head = None;
    let mut cursor = Some(snapshot.clone());
    while let Some(current) = cursor {
        if segments.len() >= MAX_GIT_SEGMENT_CHAIN_DEPTH {
            return Err(ApiError::internal_message(format!(
                "Git segment chain exceeds maximum depth of {MAX_GIT_SEGMENT_CHAIN_DEPTH}"
            )));
        }
        if is_git_segment_manifest(&current) {
            let bytes = source_blob_bytes(state.object_store.as_ref(), &current)?;
            let manifest = GitSegmentManifest::decode(&bytes)?;
            if !current.git_oid.is_empty() && manifest.head_oid != current.git_oid {
                return Err(ApiError::internal_message(
                    "Git segment manifest head does not match persisted head",
                ));
            }
            restored_head.get_or_insert(manifest.head_oid.clone());
            segments.push(manifest.segment);
            cursor = manifest.previous;
        } else {
            return Err(ApiError::internal_message(
                "raw Git cache requires a segment manifest",
            ));
        }
    }
    segments.reverse();
    for segment in segments {
        index_git_pack(state, repo_root, &segment)?;
    }
    let head_oid = restored_head
        .ok_or_else(|| ApiError::internal_message("Git segment chain did not contain a head"))?;
    run_git(
        Some(repo_root),
        &[
            "update-ref",
            &format!("refs/heads/{DEFAULT_GIT_BRANCH}"),
            &head_oid,
        ],
        "restoring Git segment head",
    )?;
    run_git(
        Some(repo_root),
        &["fsck", "--connectivity-only", &head_oid],
        "verifying restored Git segment chain",
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

fn index_git_pack(
    state: &AppState,
    repo_root: &FsPath,
    segment: &SourceBlob,
) -> Result<(), ApiError> {
    let bytes = source_blob_bytes(state.object_store.as_ref(), segment)?;
    let output = crate::git::upload::git_process_output_with_timeout(
        Command::new("git")
            .arg("--git-dir")
            .arg(repo_root)
            .args(["index-pack", "--stdin"]),
        Some(bytes),
        state.runtime_budgets.git_command_timeout(),
    )?;
    if !output.status.success() {
        return Err(ApiError::service_unavailable(format!(
            "restoring Git segment: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        )));
    }
    Ok(())
}

pub(crate) fn install_first_push_pre_receive_hook(repo_root: &FsPath) -> Result<(), ApiError> {
    let hook = repo_root.join("hooks").join("pre-receive");
    let script = format!(
        "#!/bin/sh\ncount=0\nwhile read old new ref; do\n  count=$((count + 1))\n  if [ \"$ref\" != \"refs/heads/{DEFAULT_GIT_BRANCH}\" ]; then\n    echo \"Scope accepts pushes only to refs/heads/{DEFAULT_GIT_BRANCH}\" >&2\n    exit 1\n  fi\n  if [ \"$new\" = \"{EMPTY_GIT_OID}\" ]; then\n    echo \"Scope does not accept branch deletes in v0\" >&2\n    exit 1\n  fi\n  if [ \"$old\" != \"{EMPTY_GIT_OID}\" ]; then\n    echo \"Scope accepts only the initial branch push in v0\" >&2\n    exit 1\n  fi\ndone\nif [ \"$count\" -ne 1 ]; then\n  echo \"Scope accepts exactly one pushed branch in v0\" >&2\n  exit 1\nfi\n"
    );
    write_receive_pack_hook(&hook, &script)
}

pub(crate) fn install_published_pre_receive_hook(repo_root: &FsPath) -> Result<(), ApiError> {
    let hook = repo_root.join("hooks").join("pre-receive");
    let script = format!(
        "#!/bin/sh\ncount=0\nwhile read old new ref; do\n  count=$((count + 1))\n  if [ \"$new\" = \"{EMPTY_GIT_OID}\" ]; then\n    echo \"Scope does not accept branch deletes\" >&2\n    exit 1\n  fi\n  if [ \"$ref\" = \"refs/heads/{DEFAULT_GIT_BRANCH}\" ]; then\n    if [ \"$old\" = \"{EMPTY_GIT_OID}\" ]; then\n      echo \"Scope accepts only updates to refs/heads/{DEFAULT_GIT_BRANCH}\" >&2\n      exit 1\n    fi\n    if ! git merge-base --is-ancestor \"$old\" \"$new\"; then\n      echo \"Scope rejects non-fast-forward pushes\" >&2\n      exit 1\n    fi\n    continue\n  fi\n  case \"$ref\" in\n    refs/heads/*)\n      if [ \"$old\" = \"{EMPTY_GIT_OID}\" ]; then\n        echo \"request not found; fetch before pushing\" >&2\n        exit 1\n      fi\n      if ! git cat-file -e \"$new^{{commit}}\"; then\n        echo \"Scope request refs must point at commits\" >&2\n        exit 1\n      fi\n      if ! git merge-base --is-ancestor \"$old\" \"$new\"; then\n        echo \"Scope rejects non-fast-forward request pushes\" >&2\n        exit 1\n      fi\n      ;;\n    *)\n      echo \"Scope accepts pushes only to main or a named request branch\" >&2\n      exit 1\n      ;;\n  esac\ndone\nif [ \"$count\" -ne 1 ]; then\n  echo \"Scope accepts exactly one pushed ref\" >&2\n  exit 1\nfi\n"
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
    if let Some(content_length) = content_length {
        command.env("CONTENT_LENGTH", content_length.to_string());
    }
    if let Some(content_type) = content_type {
        command.env("CONTENT_TYPE", content_type);
    }

    let mut child = command.spawn().map_err(ApiError::internal)?;
    let mut stdin = child
        .stdin
        .take()
        .ok_or_else(|| ApiError::internal_message("opening git http-backend stdin failed"))?;
    let mut output_task = tokio::spawn(async move { child.wait_with_output().await });
    let writer = async move {
        let mut stream = body.into_data_stream();
        let mut written = 0usize;
        loop {
            let next =
                tokio::time::timeout(RuntimeBudgets::default_git_command_timeout(), stream.next())
                    .await
                    .map_err(|_| ApiError::service_unavailable("git request upload stalled"))?;
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
    let request_bytes = match writer.await {
        Ok(written) => written,
        Err(error) => {
            output_task.abort();
            let _ = output_task.await;
            return Err(error);
        }
    };
    let output = match tokio::time::timeout(
        RuntimeBudgets::default_git_command_timeout(),
        &mut output_task,
    )
    .await
    {
        Ok(output) => output
            .map_err(|_| ApiError::internal_message("git http-backend task panicked"))?
            .map_err(ApiError::internal)?,
        Err(_) => {
            output_task.abort();
            let _ = output_task.await;
            return Err(ApiError::service_unavailable("git http-backend timed out"));
        }
    };
    if !output.status.success() {
        return Err(ApiError::service_unavailable(format!(
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

pub(crate) struct CgiResponse {
    pub(crate) status: StatusCode,
    pub(crate) headers: Vec<(String, String)>,
    pub(crate) body: Vec<u8>,
}

impl CgiResponse {
    pub(crate) fn parse(output: Vec<u8>) -> Result<Self, ApiError> {
        let header_end = find_header_end(&output)
            .ok_or_else(|| ApiError::service_unavailable("git http-backend returned no headers"))?;
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
                    .ok_or_else(|| ApiError::service_unavailable("invalid git CGI status"))?;
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
