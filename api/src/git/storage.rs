use crate::domain::policy::Principal;
use crate::domain::projection::project_graph;
use crate::domain::store::{RepoPublicationState, SourceBlob};
use crate::{
    config::{DEFAULT_GIT_BRANCH, EMPTY_GIT_OID, RECEIVE_PACK_STAGING_BYTES},
    error::ApiError,
    git::import::{run_git, safe_repo_key},
    git::upload::projection_bare_repo,
    object_store::source_blob_bytes,
    persistence::ensure_private_dir,
    state::{AppState, find_repo},
};
use axum::{body::Body, http::StatusCode, response::Response};
use sha2::{Digest, Sha256};
use std::{
    fs,
    io::Write,
    path::{Path as FsPath, PathBuf},
    process::{Command, Stdio},
    sync::atomic::{AtomicU64, Ordering},
};

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

pub(crate) fn git_repo_storage_root(state: &AppState) -> PathBuf {
    state.data_dir.as_ref().clone()
}

pub(crate) fn raw_git_snapshot_cache_key(snapshot: &SourceBlob) -> String {
    snapshot
        .sha256
        .get(..16)
        .unwrap_or(snapshot.sha256.as_str())
        .to_string()
}

pub(crate) fn raw_git_snapshot_cache_path(
    state: &AppState,
    snapshot: &SourceBlob,
) -> Result<PathBuf, ApiError> {
    Ok(state
        .git_cache_root()?
        .join(format!("raw-{}.git", raw_git_snapshot_cache_key(snapshot))))
}

pub(crate) fn cached_raw_git_snapshot_repo(
    state: &AppState,
    snapshot: &SourceBlob,
) -> Result<PathBuf, ApiError> {
    let repo_path = raw_git_snapshot_cache_path(state, snapshot)?;
    if repo_path
        .join("refs")
        .join("heads")
        .join(DEFAULT_GIT_BRANCH)
        .is_file()
    {
        return Ok(repo_path);
    }

    let cache_root = state.git_cache_root()?;
    let cache_key = raw_git_snapshot_cache_key(snapshot);
    let attempt = RAW_GIT_CACHE_ATTEMPT.fetch_add(1, Ordering::Relaxed);
    let temp_path = cache_root.join(format!(
        "raw-{cache_key}.{}.{}.tmp",
        std::process::id(),
        attempt
    ));
    restore_git_snapshot(state, snapshot, &temp_path)?;
    match fs::rename(&temp_path, &repo_path) {
        Ok(()) => Ok(repo_path),
        Err(error) if repo_path.exists() => {
            let _ = fs::remove_dir_all(&temp_path);
            tracing::debug!(%error, path = %repo_path.display(), "using concurrently-created raw Git snapshot cache");
            Ok(repo_path)
        }
        Err(error) => Err(ApiError::internal(error)),
    }
}

pub(crate) fn delete_raw_git_snapshot_cache(
    state: &AppState,
    snapshot: &SourceBlob,
) -> Result<(), ApiError> {
    let cache_root = state.git_cache_root()?;
    let cache_key = raw_git_snapshot_cache_key(snapshot);
    remove_dir_if_exists(&cache_root.join(format!("raw-{cache_key}.git")))?;

    let entries = fs::read_dir(&cache_root).map_err(ApiError::internal)?;
    let temp_prefix = format!("raw-{cache_key}.");
    for entry in entries {
        let entry = entry.map_err(ApiError::internal)?;
        let file_name = entry.file_name();
        let file_name = file_name.to_string_lossy();
        if file_name.starts_with(&temp_prefix) && file_name.ends_with(".tmp") {
            remove_dir_if_exists(&entry.path())?;
        }
    }

    Ok(())
}

pub(crate) fn delete_repo_storage(
    state: &AppState,
    owner: &str,
    repo_name: &str,
) -> Result<(), ApiError> {
    remove_dir_if_exists(&owner_git_repo_path(state, owner, repo_name))?;
    remove_dir_if_exists(&staged_git_repo_path(state, owner, repo_name))?;

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

pub(crate) fn ensure_published_receive_pack_staging_repo(
    state: &AppState,
    owner: &str,
    repo_name: &str,
    author_id: &str,
) -> Result<PathBuf, ApiError> {
    let repo = find_repo(state, owner, repo_name)?;
    if repo.record.publication_state != RepoPublicationState::Published {
        return Err(ApiError::conflict("repo must be published before push"));
    }
    let principal = Principal {
        id: author_id.to_string(),
        kind: crate::domain::policy::PrincipalKind::User,
    };
    let repo_root = receive_pack_staging_repo_path(state, owner, repo_name)?;
    if let Some(parent) = repo_root.parent() {
        ensure_private_dir(parent)?;
    }
    if let Some(snapshot) = repo.git_snapshot.as_ref() {
        let seed_repo = cached_raw_git_snapshot_repo(state, snapshot)?;
        let seed = seed_repo.to_string_lossy().to_string();
        let target = repo_root.to_string_lossy().to_string();
        run_git(
            None,
            &["clone", "--bare", "--no-hardlinks", &seed, &target],
            "cloning receive-pack staging repo",
        )?;
    } else {
        let projection = project_graph(&repo.policy, &repo.graph, &principal);
        let seed_repo = projection_bare_repo(
            state.object_store.as_ref(),
            &state.git_cache_root()?,
            &projection,
        )?;
        let seed = seed_repo.to_string_lossy().to_string();
        let target = repo_root.to_string_lossy().to_string();
        run_git(
            None,
            &["clone", "--bare", "--no-hardlinks", &seed, &target],
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

pub(crate) fn restore_git_snapshot(
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
    let bundle_path = repo_root.with_extension(format!(
        "bundle.{}.tmp",
        hex::encode(&snapshot.sha256.as_bytes()[..8])
    ));
    let bytes = source_blob_bytes(state.object_store.as_ref(), snapshot)?;
    fs::write(&bundle_path, bytes).map_err(ApiError::internal)?;
    let bundle = bundle_path.to_string_lossy().to_string();
    run_git(
        Some(repo_root),
        &[
            "fetch",
            &bundle,
            &format!("refs/heads/{DEFAULT_GIT_BRANCH}:refs/heads/{DEFAULT_GIT_BRANCH}"),
        ],
        "restoring Git snapshot",
    )?;
    let _ = fs::remove_file(&bundle_path);
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
        "#!/bin/sh\ncount=0\nwhile read old new ref; do\n  count=$((count + 1))\n  if [ \"$ref\" != \"refs/heads/{DEFAULT_GIT_BRANCH}\" ]; then\n    echo \"Scope accepts pushes only to refs/heads/{DEFAULT_GIT_BRANCH}\" >&2\n    exit 1\n  fi\n  if [ \"$new\" = \"{EMPTY_GIT_OID}\" ]; then\n    echo \"Scope does not accept branch deletes in v0\" >&2\n    exit 1\n  fi\n  if [ \"$old\" = \"{EMPTY_GIT_OID}\" ]; then\n    echo \"Scope accepts only updates to refs/heads/{DEFAULT_GIT_BRANCH}\" >&2\n    exit 1\n  fi\n  if ! git merge-base --is-ancestor \"$old\" \"$new\"; then\n    echo \"Scope rejects non-fast-forward pushes in v0\" >&2\n    exit 1\n  fi\ndone\nif [ \"$count\" -ne 1 ]; then\n  echo \"Scope accepts exactly one pushed branch in v0\" >&2\n  exit 1\nfi\n"
    );
    write_receive_pack_hook(&hook, &script)
}

pub(crate) fn write_receive_pack_hook(hook: &FsPath, script: &str) -> Result<(), ApiError> {
    fs::write(&hook, script).map_err(ApiError::internal)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut permissions = fs::metadata(&hook)
            .map_err(ApiError::internal)?
            .permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(&hook, permissions).map_err(ApiError::internal)?;
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

    let mut child = command.spawn().map_err(|error| {
        ApiError::service_unavailable(format!("failed to start git http-backend: {error}"))
    })?;
    if let Some(stdin) = child.stdin.as_mut() {
        stdin.write_all(&body).map_err(ApiError::internal)?;
    }

    let output = child.wait_with_output().map_err(ApiError::internal)?;
    if !output.status.success() {
        return Err(ApiError::service_unavailable(format!(
            "git http-backend failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        )));
    }

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
                    .trim()
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
