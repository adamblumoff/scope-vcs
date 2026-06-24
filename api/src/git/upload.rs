use crate::domain::git_projection::projection_blob_text;
use crate::domain::policy::Principal;
use crate::domain::projection::{Projection, project_graph};
use crate::domain::store::{RepoPublicationState, RepoRole};
use crate::{
    auth::shoo::{http_identity, principal_for_repo},
    config::{DEFAULT_GIT_BRANCH, GIT_UPLOAD_PACK, UNPUBLISHED_GIT_ERROR},
    error::ApiError,
    git::{
        GitReadAuthorization, authorize_git_clone_token_for_repo,
        authorize_git_push_token_for_repo, find_repo_after_git_scope_token, git_credential_error,
        git_read_authorization_from_headers, invalid_git_credentials,
        storage::cached_raw_git_snapshot_repo,
    },
    state::AppState,
    state::{ensure_repo_read, find_repo, role_for_principal},
};
use axum::{
    body::Body,
    http::{
        HeaderMap, StatusCode,
        header::{AUTHORIZATION, CACHE_CONTROL, CONTENT_TYPE},
    },
    response::{IntoResponse, Response},
};
use sha1::{Digest, Sha1};
use std::{
    collections::BTreeMap,
    fs,
    io::Write,
    path::{Path as FsPath, PathBuf},
    process::{Command, Stdio},
    sync::atomic::{AtomicU64, Ordering},
};
use tokio::{io::AsyncWriteExt, process::Command as TokioCommand};

static GIT_CACHE_ATTEMPT: AtomicU64 = AtomicU64::new(1);

pub(crate) async fn git_projection_for_request(
    state: &AppState,
    headers: &HeaderMap,
    owner: &str,
    repo_name: &str,
) -> Result<Projection, ApiError> {
    if let Some(read_auth) = git_read_authorization_from_headers(headers)? {
        let (repo, principal) = principal_for_git_read_token(state, read_auth, owner, repo_name)?;
        if repo.record.publication_state != RepoPublicationState::Published {
            return match role_for_principal(state, &repo, &principal)? {
                Some(RepoRole::Owner) => Err(ApiError::forbidden(UNPUBLISHED_GIT_ERROR)),
                _ => Err(ApiError::not_found(format!(
                    "repo {owner}/{repo_name} not found"
                ))),
            };
        }

        ensure_repo_read(state, &repo, &principal)?;
        return Ok(project_graph(&repo.policy, &repo.graph, &principal));
    }

    let repo = find_repo(state, owner, repo_name)?;
    let identity = http_identity(state, headers).await?;
    let principal = principal_for_repo(state, &repo, identity.as_ref())?;
    if repo.record.publication_state != RepoPublicationState::Published {
        return match role_for_principal(state, &repo, &principal)? {
            Some(RepoRole::Owner) => Err(ApiError::forbidden(UNPUBLISHED_GIT_ERROR)),
            _ => Err(ApiError::not_found(format!(
                "repo {owner}/{repo_name} not found"
            ))),
        };
    }

    ensure_repo_read(state, &repo, &principal)?;
    Ok(project_graph(&repo.policy, &repo.graph, &principal))
}

pub(crate) async fn git_upload_pack_repo_for_request(
    state: &AppState,
    headers: &HeaderMap,
    owner: &str,
    repo_name: &str,
) -> Result<PathBuf, ApiError> {
    if headers.contains_key(AUTHORIZATION)
        && let Some(repo_path) =
            owner_snapshot_repo_for_request(state, headers, owner, repo_name).await?
    {
        return Ok(repo_path);
    }

    let projection = match git_projection_for_request(state, headers, owner, repo_name).await {
        Ok(projection) => projection,
        Err(error)
            if !headers.contains_key(AUTHORIZATION) && error.status == StatusCode::NOT_FOUND =>
        {
            return Err(git_upload_pack_auth_required());
        }
        Err(error) => return Err(error),
    };
    state.git_cache_root().and_then(|cache_root| {
        projection_bare_repo(state.object_store.as_ref(), &cache_root, &projection)
    })
}

pub(crate) fn git_upload_pack_auth_required() -> ApiError {
    ApiError::unauthorized("Git credentials required")
}

pub(crate) async fn owner_snapshot_repo_for_request(
    state: &AppState,
    headers: &HeaderMap,
    owner: &str,
    repo_name: &str,
) -> Result<Option<PathBuf>, ApiError> {
    let (repo, principal) = if let Some(read_auth) = git_read_authorization_from_headers(headers)? {
        principal_for_git_read_token(state, read_auth, owner, repo_name)?
    } else {
        let repo = find_repo(state, owner, repo_name)?;
        let identity = http_identity(state, headers).await?;
        let principal = principal_for_repo(state, &repo, identity.as_ref())?;
        (repo, principal)
    };
    if role_for_principal(state, &repo, &principal)? != Some(RepoRole::Owner) {
        return Ok(None);
    }
    if repo.record.publication_state != RepoPublicationState::Published {
        return Err(ApiError::forbidden(UNPUBLISHED_GIT_ERROR));
    }
    let Some(snapshot) = repo
        .staged_update
        .as_ref()
        .map(|update| &update.git_snapshot)
        .or(repo.git_snapshot.as_ref())
    else {
        return Ok(None);
    };
    cached_raw_git_snapshot_repo(state, snapshot).map(Some)
}

fn principal_for_git_read_token(
    state: &AppState,
    read_auth: GitReadAuthorization,
    owner: &str,
    repo_name: &str,
) -> Result<(crate::domain::store::StoredRepository, Principal), ApiError> {
    let repo = find_repo_after_git_scope_token(state, owner, repo_name)?;
    let user_id = match read_auth {
        GitReadAuthorization::PushToken { secret } => {
            authorize_git_push_token_for_repo(&repo, &secret).map_err(git_credential_error)?
        }
        GitReadAuthorization::CloneToken { secret } => {
            let user_id =
                authorize_git_clone_token_for_repo(&repo, &secret).map_err(git_credential_error)?;
            let principal = Principal {
                id: user_id.clone(),
                kind: crate::domain::policy::PrincipalKind::User,
            };
            if role_for_principal(state, &repo, &principal)?.is_none() {
                return Err(invalid_git_credentials());
            }
            user_id
        }
    };

    Ok((
        repo,
        Principal {
            id: user_id,
            kind: crate::domain::policy::PrincipalKind::User,
        },
    ))
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
                    visible_tree.insert(path, projection_blob_text(store, blob)?);
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

pub(crate) fn projection_cache_key(projection: &Projection) -> String {
    let mut hasher = Sha1::new();
    hash_field(&mut hasher, b"repo", projection.repo_id.as_bytes());
    hash_field(
        &mut hasher,
        b"principal",
        projection.principal_id.as_bytes(),
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
    visible_tree: &BTreeMap<String, String>,
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
    for (path, content) in visible_tree {
        let oid = git_command_output(
            Command::new("git")
                .arg("--git-dir")
                .arg(repo_path)
                .arg("hash-object")
                .arg("-w")
                .arg("--stdin"),
            Some(content.as_bytes()),
        )?;
        let oid = String::from_utf8(oid).map_err(ApiError::bad_request)?;
        let relative_path = git_relative_path(path)?;
        index_info
            .extend_from_slice(format!("100644 blob {}\t{relative_path}\n", oid.trim()).as_bytes());
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
    if stdin.is_some() {
        command.stdin(Stdio::piped());
    }
    let mut child = command
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| ApiError::service_unavailable(format!("failed to run Git: {error}")))?;
    if let Some(input) = stdin {
        let mut child_stdin = child
            .stdin
            .take()
            .ok_or_else(|| ApiError::internal_message("failed to open Git stdin"))?;
        child_stdin.write_all(input).map_err(ApiError::internal)?;
    }
    let output = child.wait_with_output().map_err(ApiError::internal)?;
    if output.status.success() {
        return Ok(output.stdout);
    }

    let stderr = String::from_utf8_lossy(&output.stderr);
    Err(ApiError::service_unavailable(stderr.trim()))
}

pub(crate) async fn git_upload_pack_response(
    repo_path: &FsPath,
    request: &[u8],
) -> Result<Response, ApiError> {
    let mut child = TokioCommand::new("git")
        .arg("upload-pack")
        .arg("--stateless-rpc")
        .arg(repo_path)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| ApiError::service_unavailable(format!("failed to run Git: {error}")))?;

    let mut stdin = child
        .stdin
        .take()
        .ok_or_else(|| ApiError::internal_message("failed to open Git stdin"))?;
    stdin.write_all(request).await.map_err(ApiError::internal)?;
    drop(stdin);

    let output = child.wait_with_output().await.map_err(ApiError::internal)?;
    if !output.status.success() {
        return Err(ApiError::service_unavailable(format!(
            "git upload-pack failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        )));
    }

    Ok(git_response(
        "application/x-git-upload-pack-result",
        output.stdout,
    ))
}

pub(crate) fn git_upload_pack_advertisement(repo_path: &FsPath) -> Response {
    match git_command_output(
        Command::new("git")
            .arg("upload-pack")
            .arg("--stateless-rpc")
            .arg("--advertise-refs")
            .arg(repo_path),
        None,
    ) {
        Ok(advertisement) => {
            let mut body = pkt_line(format!("# service={GIT_UPLOAD_PACK}\n").as_bytes());
            body.extend_from_slice(b"0000");
            body.extend(advertisement);
            git_response("application/x-git-upload-pack-advertisement", body)
        }
        Err(error) => git_advertisement_error(error.message),
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
