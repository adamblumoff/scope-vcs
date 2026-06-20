use crate::domain::policy::Principal;
use crate::domain::projection::project_graph;
use crate::domain::store::RepoPublicationState;
use crate::{
    config::{DEFAULT_GIT_BRANCH, EMPTY_GIT_OID, RECEIVE_PACK_STAGING_BYTES},
    error::ApiError,
    git::import::{run_git, safe_repo_key},
    git::upload::projection_bare_repo,
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

#[cfg(test)]
pub(crate) fn replace_git_repo(src: &FsPath, dst: &FsPath) -> Result<(), ApiError> {
    if let Some(parent) = dst.parent() {
        ensure_private_dir(parent)?;
    }
    if dst.exists() {
        fs::remove_dir_all(dst).map_err(ApiError::internal)?;
    }
    fs::rename(src, dst).map_err(ApiError::internal)
}

pub(crate) fn replace_git_repo_and_then<T>(
    src: &FsPath,
    dst: &FsPath,
    op: impl FnOnce() -> Result<T, ApiError>,
) -> Result<T, ApiError> {
    let replacement = begin_git_repo_replacement(src, dst)?;

    match op() {
        Ok(value) => {
            replacement.commit()?;
            Ok(value)
        }
        Err(error) => Err(error),
    }
}

pub(crate) struct GitRepoReplacement {
    src: PathBuf,
    dst: PathBuf,
    backup: PathBuf,
    active: bool,
}

impl GitRepoReplacement {
    pub(crate) fn commit(mut self) -> Result<(), ApiError> {
        self.active = false;
        if self.backup.exists() {
            fs::remove_dir_all(&self.backup).map_err(ApiError::internal)?;
        }
        Ok(())
    }

    fn rollback(&mut self) {
        let _ = fs::rename(&self.dst, &self.src);
        if self.backup.exists() {
            let _ = fs::rename(&self.backup, &self.dst);
        }
        self.active = false;
    }
}

impl Drop for GitRepoReplacement {
    fn drop(&mut self) {
        if self.active {
            self.rollback();
        }
    }
}

pub(crate) fn begin_git_repo_replacement(
    src: &FsPath,
    dst: &FsPath,
) -> Result<GitRepoReplacement, ApiError> {
    if let Some(parent) = dst.parent() {
        ensure_private_dir(parent)?;
    }
    let backup = unique_sibling_path(dst, "backup")?;
    if backup.exists() {
        fs::remove_dir_all(&backup).map_err(ApiError::internal)?;
    }
    if dst.exists() {
        fs::rename(dst, &backup).map_err(ApiError::internal)?;
    }
    fs::rename(src, dst).map_err(|error| {
        if backup.exists() {
            let _ = fs::rename(&backup, dst);
        }
        ApiError::internal(error)
    })?;

    Ok(GitRepoReplacement {
        src: src.to_path_buf(),
        dst: dst.to_path_buf(),
        backup,
        active: true,
    })
}

pub(crate) fn remove_git_repo_and_then<T>(
    path: &FsPath,
    op: impl FnOnce() -> Result<T, ApiError>,
) -> Result<T, ApiError> {
    let backup = unique_sibling_path(path, "delete")?;
    fs::rename(path, &backup).map_err(ApiError::internal)?;
    match op() {
        Ok(value) => {
            fs::remove_dir_all(&backup).map_err(ApiError::internal)?;
            Ok(value)
        }
        Err(error) => {
            let _ = fs::rename(&backup, path);
            Err(error)
        }
    }
}

pub(crate) fn unique_sibling_path(path: &FsPath, label: &str) -> Result<PathBuf, ApiError> {
    let parent = path
        .parent()
        .ok_or_else(|| ApiError::internal_message("git repo path is missing a parent"))?;
    let name = path
        .file_name()
        .and_then(|value| value.to_str())
        .ok_or_else(|| ApiError::internal_message("git repo path has invalid file name"))?;
    let mut bytes = [0_u8; RECEIVE_PACK_STAGING_BYTES];
    getrandom::fill(&mut bytes).map_err(|error| {
        ApiError::internal_message(format!("failed to create git repo backup path: {error}"))
    })?;
    Ok(parent.join(format!("{name}.{label}.{}", hex::encode(bytes))))
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
    let owner_repo = owner_git_repo_path(state, owner, repo_name);
    let seed_repo = if owner_repo.join("HEAD").exists() {
        owner_repo
    } else {
        let projection = project_graph(&repo.policy, &repo.graph, &principal);
        projection_bare_repo(&state.git_cache_root()?, &projection)?
    };
    let repo_root = receive_pack_staging_repo_path(state, owner, repo_name)?;
    if let Some(parent) = repo_root.parent() {
        ensure_private_dir(parent)?;
    }
    let seed = seed_repo.to_string_lossy().to_string();
    let target = repo_root.to_string_lossy().to_string();
    run_git(
        None,
        &["clone", "--bare", "--no-hardlinks", &seed, &target],
        "cloning receive-pack staging repo",
    )?;
    run_git(
        Some(&repo_root),
        &["config", "http.receivepack", "true"],
        "enabling receive-pack",
    )?;
    install_published_pre_receive_hook(&repo_root)?;
    Ok(repo_root)
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
        "#!/bin/sh\ncount=0\nwhile read old new ref; do\n  count=$((count + 1))\n  if [ \"$ref\" != \"refs/heads/{DEFAULT_GIT_BRANCH}\" ]; then\n    echo \"Scope accepts pushes only to refs/heads/{DEFAULT_GIT_BRANCH}\" >&2\n    exit 1\n  fi\n  if [ \"$new\" = \"{EMPTY_GIT_OID}\" ]; then\n    echo \"Scope does not accept branch deletes in v0\" >&2\n    exit 1\n  fi\n  if [ \"$old\" = \"{EMPTY_GIT_OID}\" ]; then\n    echo \"Scope accepts only updates to refs/heads/{DEFAULT_GIT_BRANCH}\" >&2\n    exit 1\n  fi\ndone\nif [ \"$count\" -ne 1 ]; then\n  echo \"Scope accepts exactly one pushed branch in v0\" >&2\n  exit 1\nfi\n"
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
