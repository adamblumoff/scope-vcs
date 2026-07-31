use crate::{
    auth::scope::principal_for_user_id,
    config::{DEFAULT_GIT_BRANCH, GIT_UPLOAD_PACK, UNPUBLISHED_GIT_ERROR},
    error::ApiError,
    git::{
        GitRemoteMode,
        cache::{GitDerivedCacheNamespace, GitRepoHandle},
        git_read_scope_user,
        import::run_git,
        projection_repo::{hash_field, projection_bare_repo_for_state},
        request_refs::attach_visible_request_refs,
        storage::cached_raw_git_repo,
    },
    repo_access::{ensure_repo_read, find_repo},
    runtime_budgets::{RuntimeBudgets, RuntimePermit},
    state::AppState,
};
use axum::{
    body::Body,
    http::{
        HeaderMap, StatusCode,
        header::{CACHE_CONTROL, CONTENT_TYPE},
    },
    response::{IntoResponse, Response},
};
use scope_domain::policy::Principal;
#[cfg(test)]
use scope_domain::projection::Projection;
use scope_domain::projection::{ProjectionViewKey, project_graph};
use scope_domain::requests::{Request, RequestViewer, canonical_request_ref, request_policy};
use scope_domain::store::{RepoPublicationState, RepositoryActor};
use scope_git_process::{
    ProcessLimits, STDERR_DIAGNOSTIC_BYTES, run as run_process, truncated_stderr,
};
use sha1::{Digest, Sha1};
use std::{
    fs,
    path::Path as FsPath,
    process::{Command, Output},
    sync::atomic::{AtomicU64, Ordering},
    time::Duration,
};
const GIT_READ_VIEW_CACHE_SEMANTICS_VERSION: &str = "named-request-read-view-v2";
static GIT_READ_VIEW_CACHE_ATTEMPT: AtomicU64 = AtomicU64::new(1);

#[cfg(test)]
pub(crate) async fn git_projection_for_request(
    state: &AppState,
    headers: &HeaderMap,
    owner: &str,
    repo_name: &str,
    mode: GitRemoteMode,
) -> Result<Projection, ApiError> {
    let (repo, principal, _) =
        git_read_principal_for_request(state, headers, owner, repo_name, mode).await?;
    if repo.record.publication_state != RepoPublicationState::Published {
        return Err(unpublished_git_read_error(
            &repo, owner, repo_name, &principal,
        ));
    }

    ensure_repo_read(state, &repo, &principal)?;
    let access = repo.access_for_principal(&principal);
    let view_key = ProjectionViewKey::from_access(access);
    Ok(project_graph(
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
) -> Result<GitRepoHandle, ApiError> {
    let (repo, principal, viewer_user_id) =
        match git_read_principal_for_request(state, headers, owner, repo_name, mode).await {
            Ok(value) => value,
            Err(error)
                if mode == GitRemoteMode::Public && error.status() == StatusCode::NOT_FOUND =>
            {
                return Err(git_upload_pack_auth_required());
            }
            Err(error) => return Err(error),
        };
    if repo.record.publication_state != RepoPublicationState::Published {
        return Err(unpublished_git_read_error(
            &repo, owner, repo_name, &principal,
        ));
    }
    ensure_repo_read(state, &repo, &principal)?;
    let access = repo.access_for_principal(&principal);
    let private_view = ProjectionViewKey::from_access(access) == ProjectionViewKey::Private;
    let base_repo = if private_view {
        match repo.git_head.as_ref() {
            Some(head) => cached_raw_git_repo(state, &head.manifest)?,
            None => {
                let projection = project_graph(
                    &repo.graph,
                    &repo.visibility_events,
                    ProjectionViewKey::Private,
                );
                GitRepoHandle::from_path(projection_bare_repo_for_state(
                    state,
                    &projection,
                    repo.git_head.as_ref().map(|head| &head.manifest),
                )?)
            }
        }
    } else {
        let projection = project_graph(
            &repo.graph,
            &repo.visibility_events,
            ProjectionViewKey::Public,
        );
        GitRepoHandle::from_path(projection_bare_repo_for_state(
            state,
            &projection,
            repo.git_head.as_ref().map(|head| &head.manifest),
        )?)
    };
    let mut requests = Vec::new();
    let mut hidden_request_refs = Vec::new();
    for request in state
        .metadata
        .requests()
        .requests_by_repo_id(&repo.record.id)
        .await?
    {
        let is_invitee = match viewer_user_id.as_deref() {
            Some(user_id) => {
                state
                    .metadata
                    .requests()
                    .request_is_invitee(&request.id, user_id)
                    .await?
            }
            None => false,
        };
        let decision = request_policy(
            &request,
            RequestViewer::new(access, viewer_user_id.as_deref(), is_invitee),
        );
        if decision.request_ref_readable {
            if !decision.git_advertised {
                hidden_request_refs.push(request.name.clone());
            }
            requests.push(request);
        }
    }
    requests.sort_by(|left, right| left.name.cmp(&right.name));
    let public_base_repo = if private_view
        && requests.iter().any(|request| {
            request.audience == scope_domain::requests::RequestAudience::Public
                && request.git_snapshot.is_none()
        }) {
        let projection = project_graph(
            &repo.graph,
            &repo.visibility_events,
            ProjectionViewKey::Public,
        );
        Some(projection_bare_repo_for_state(
            state,
            &projection,
            repo.git_head.as_ref().map(|head| &head.manifest),
        )?)
    } else {
        None
    };
    git_read_view_repo(
        state,
        base_repo,
        public_base_repo.as_deref(),
        viewer_user_id.as_deref(),
        &requests,
        &hidden_request_refs,
    )
}

fn git_read_view_repo(
    state: &AppState,
    base_repo: GitRepoHandle,
    public_base_repo: Option<&FsPath>,
    viewer_user_id: Option<&str>,
    requests: &[Request],
    hidden_request_refs: &[String],
) -> Result<GitRepoHandle, ApiError> {
    if requests.is_empty() {
        return Ok(base_repo);
    }
    let main_oid = git_command_output(
        Command::new("git")
            .arg("--git-dir")
            .arg(base_repo.as_ref())
            .arg("rev-parse")
            .arg(format!("refs/heads/{DEFAULT_GIT_BRANCH}")),
        None,
    )?;
    let mut hasher = Sha1::new();
    hash_field(
        &mut hasher,
        b"semantics",
        GIT_READ_VIEW_CACHE_SEMANTICS_VERSION.as_bytes(),
    );
    hash_field(&mut hasher, b"main", &main_oid);
    match viewer_user_id {
        Some(user_id) => {
            hash_field(&mut hasher, b"authorization", b"user");
            hash_field(&mut hasher, b"viewer", user_id.as_bytes());
        }
        None => hash_field(&mut hasher, b"authorization", b"public"),
    }
    for request in requests {
        hash_field(&mut hasher, b"name", request.name.as_bytes());
        hash_field(&mut hasher, b"head", request.head_oid.as_bytes());
        hash_field(
            &mut hasher,
            b"audience",
            format!("{:?}", request.audience).as_bytes(),
        );
        hash_field(
            &mut hasher,
            b"state",
            format!("{:?}", request.state).as_bytes(),
        );
        if let Some(snapshot) = request.git_snapshot.as_ref() {
            hash_field(&mut hasher, b"snapshot", snapshot.sha256.as_bytes());
        }
    }
    for request_name in hidden_request_refs {
        hash_field(&mut hasher, b"hidden", request_name.as_bytes());
    }
    let cache_key = hex::encode(hasher.finalize());
    let cache_root = state.git_cache_root()?;
    let repo_path = cache_root.join(format!("read-view-{cache_key}.git"));
    let is_ready = || repo_path.join("objects").is_dir();
    state.git_cache_builds.materialize(
        GitDerivedCacheNamespace::RequestReadView,
        cache_key.clone(),
        is_ready,
        || {
            let _permit = state.runtime_budgets.try_projection_build()?;
            let attempt = GIT_READ_VIEW_CACHE_ATTEMPT.fetch_add(1, Ordering::Relaxed);
            let temp_path = cache_root.join(format!(
                "read-view-{cache_key}.{}.{}.tmp",
                std::process::id(),
                attempt
            ));
            if temp_path.exists() {
                fs::remove_dir_all(&temp_path).map_err(ApiError::internal)?;
            }
            let result = (|| {
                git_command_output(
                    Command::new("git")
                        .arg("clone")
                        .arg("--bare")
                        .arg("--no-hardlinks")
                        .arg(base_repo.as_ref())
                        .arg(&temp_path),
                    None,
                )?;
                attach_visible_request_refs(state, requests, &temp_path, public_base_repo)?;
                if !hidden_request_refs.is_empty() {
                    run_git(
                        Some(&temp_path),
                        &["config", "uploadpack.allowTipSHA1InWant", "true"],
                        "allowing exact request tip fetches",
                    )?;
                    for request_name in hidden_request_refs {
                        run_git(
                            Some(&temp_path),
                            &[
                                "config",
                                "--add",
                                "transfer.hideRefs",
                                &canonical_request_ref(request_name),
                            ],
                            "hiding exact-only request ref from advertisement",
                        )?;
                    }
                }
                match fs::rename(&temp_path, &repo_path) {
                    Ok(()) => Ok(()),
                    Err(error) if is_ready() => {
                        tracing::debug!(%error, path = %repo_path.display(), "using externally-created Git read view cache");
                        Ok(())
                    }
                    Err(error) => Err(ApiError::internal(error)),
                }
            })();
            let _ = fs::remove_dir_all(&temp_path);
            result
        },
    )?;
    Ok(GitRepoHandle::from_path(repo_path))
}

pub(crate) fn git_upload_pack_auth_required() -> ApiError {
    ApiError::unauthorized("Git credentials required")
}

async fn git_read_principal_for_request(
    state: &AppState,
    headers: &HeaderMap,
    owner: &str,
    repo_name: &str,
    mode: GitRemoteMode,
) -> Result<
    (
        scope_domain::store::StoredRepository,
        Principal,
        Option<String>,
    ),
    ApiError,
> {
    match mode {
        GitRemoteMode::Public => {
            let repo = find_repo(state, owner, repo_name).await?;
            Ok((repo, Principal::public(), None))
        }
        GitRemoteMode::Permissioned => {
            let user = git_read_scope_user(state, headers).await?;
            let repo = find_repo(state, owner, repo_name).await?;
            let principal = principal_for_user_id(&repo, &user.id);
            Ok((repo, principal, Some(user.id)))
        }
    }
}

fn unpublished_git_read_error(
    repo: &scope_domain::store::StoredRepository,
    owner: &str,
    repo_name: &str,
    principal: &Principal,
) -> ApiError {
    if repo.access_for_principal(principal).actor == RepositoryActor::Owner {
        ApiError::forbidden(UNPUBLISHED_GIT_ERROR)
    } else {
        ApiError::not_found(format!("repo {owner}/{repo_name} not found"))
    }
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
    git_process_output(command, stdin, ProcessLimits::new(timeout))
}

pub(crate) fn git_process_output_with_limits(
    command: &mut Command,
    stdin: Option<Vec<u8>>,
    timeout: Duration,
    max_stdout_bytes: usize,
) -> Result<Output, ApiError> {
    run_process(
        command,
        stdin,
        ProcessLimits::new(timeout).with_max_stdout_bytes(max_stdout_bytes),
        "Git command",
    )
    .map_err(|error| {
        if error.is_stdout_limit() {
            ApiError::payload_too_large(error.to_string())
        } else {
            ApiError::service_unavailable(error.to_string())
        }
    })
}

fn git_process_output(
    command: &mut Command,
    stdin: Option<Vec<u8>>,
    limits: ProcessLimits,
) -> Result<Output, ApiError> {
    run_process(command, stdin, limits, "Git command")
        .map_err(|error| ApiError::service_unavailable(error.to_string()))
}

pub(crate) fn truncated_git_stderr(stderr: &[u8]) -> String {
    truncated_stderr(stderr, STDERR_DIAGNOSTIC_BYTES)
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
    use std::time::Instant;

    #[test]
    fn stderr_truncation_preserves_utf8_boundaries() {
        let stderr = "é".repeat(STDERR_DIAGNOSTIC_BYTES);

        let truncated = truncated_git_stderr(stderr.as_bytes());

        assert!(truncated.ends_with("..."));
        assert!(truncated.is_char_boundary(truncated.len() - 3));
    }

    #[cfg(unix)]
    #[test]
    fn bounded_git_output_maps_size_limit_to_payload_too_large() {
        let mut command = Command::new("sh");
        command.arg("-c").arg("printf 12345");

        let error = git_process_output_with_limits(&mut command, None, Duration::from_secs(1), 4)
            .unwrap_err();

        assert_eq!(error.status(), StatusCode::PAYLOAD_TOO_LARGE);
        assert!(error.message().contains("stdout exceeded 4 bytes"));
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
