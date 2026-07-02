mod credentials;
pub(crate) mod import;
pub(crate) mod storage;
pub(crate) mod upload;

pub(crate) use credentials::*;

use crate::domain::store::{RepoPublicationState, RepositoryActor};
use crate::{
    auth::scope::principal_for_user_id,
    config::*,
    error::ApiError,
    git::{
        import::{
            git_refs, pending_import_from_staging_repo, persist_pending_import,
            persist_receive_pack_update_and_promote, receive_pack_update_from_staging_repo,
        },
        storage::*,
        upload::*,
    },
    state::AppState,
    state::find_repo,
};
use axum::{
    Json,
    body::{Bytes, to_bytes},
    extract::{Path, Query, Request, State},
    http::{
        HeaderMap, HeaderValue, StatusCode,
        header::{CONTENT_ENCODING, CONTENT_TYPE, WWW_AUTHENTICATE},
    },
    response::{IntoResponse, Response},
};
use flate2::read::GzDecoder;
use serde::Deserialize;
use std::{fs, io::Read, time::Instant};

#[derive(Debug, Deserialize)]
pub(crate) struct GitInfoRefsQuery {
    pub(crate) service: Option<String>,
}

#[derive(Debug)]
pub(crate) enum ReceivePackAccess {
    FirstPush { credential: InitialPushCredential },
    PublishedMember { author_id: String },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum PersistedReceivePackUpdate {
    Staged,
    Applied,
}

pub(crate) fn git_error_response(error: ApiError) -> Response {
    if error.status == StatusCode::UNAUTHORIZED {
        let mut response = error.into_response();
        response.headers_mut().insert(
            WWW_AUTHENTICATE,
            HeaderValue::from_static("Basic realm=\"Scope Git\""),
        );
        return response;
    }
    error.into_response()
}

pub(crate) async fn git_info_refs(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((org, repo)): Path<(String, String)>,
    Query(query): Query<GitInfoRefsQuery>,
) -> Response {
    match query.service.as_deref() {
        Some(GIT_RECEIVE_PACK) => match receive_pack_access(&state, &headers, &org, &repo).await {
            Ok(access) => {
                let _permit = match state.runtime_budgets.try_receive_pack() {
                    Ok(permit) => permit,
                    Err(error) => return git_error_response(error),
                };
                match handle_git_receive_pack(&state, &org, &repo, "GET", Vec::new(), None, access)
                {
                    Ok(response) => response,
                    Err(error) => git_error_response(error),
                }
            }
            Err(error) => git_error_response(error),
        },
        Some(GIT_UPLOAD_PACK) => {
            let _permit = match state.runtime_budgets.try_upload_pack() {
                Ok(permit) => permit,
                Err(error) => return git_advertisement_error(error.message),
            };
            match git_upload_pack_repo_for_request(&state, &headers, &org, &repo).await {
                Ok(repo_path) => git_upload_pack_advertisement(
                    &repo_path,
                    state.runtime_budgets.git_command_timeout(),
                ),
                Err(error) if error.status == StatusCode::UNAUTHORIZED => git_error_response(error),
                Err(error) => git_advertisement_error(error.message),
            }
        }
        Some(service) => (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({
                "error": format!("unsupported Git service {service}")
            })),
        )
            .into_response(),
        None => (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({
                "error": "missing Git service"
            })),
        )
            .into_response(),
    }
}

pub(crate) async fn git_receive_pack(
    State(state): State<AppState>,
    Path((org, repo)): Path<(String, String)>,
    request: Request,
) -> Response {
    let headers = request.headers().clone();
    let access = match receive_pack_access(&state, &headers, &org, &repo).await {
        Ok(access) => access,
        Err(error) => return git_error_response(error),
    };
    let _permit = match state.runtime_budgets.try_receive_pack() {
        Ok(permit) => permit,
        Err(error) => return git_error_response(error),
    };

    let content_type = headers
        .get(CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .map(str::to_string);
    let body = match to_bytes(request.into_body(), MAX_RECEIVE_PACK_BYTES).await {
        Ok(body) => body,
        Err(error) => {
            return git_error_response(ApiError::payload_too_large(format!(
                "git receive-pack body is too large: {error}"
            )));
        }
    };
    let body = match decode_git_request_body(&headers, body, MAX_RECEIVE_PACK_BYTES) {
        Ok(body) => body,
        Err(error) => return git_error_response(error),
    };

    match handle_git_receive_pack(&state, &org, &repo, "POST", body, content_type, access) {
        Ok(response) => response,
        Err(error) => git_error_response(error),
    }
}

pub(crate) async fn git_upload_pack_rpc(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((org, repo_name)): Path<(String, String)>,
    request: Request,
) -> Response {
    let permit = match state.runtime_budgets.try_upload_pack() {
        Ok(permit) => permit,
        Err(error) => return git_upload_pack_error(error.message),
    };
    let repo_path = match git_upload_pack_repo_for_request(&state, &headers, &org, &repo_name).await
    {
        Ok(repo_path) => repo_path,
        Err(error) => return git_upload_pack_error(error.message),
    };
    let body = match to_bytes(request.into_body(), MAX_UPLOAD_PACK_BYTES).await {
        Ok(body) => body,
        Err(error) => {
            return git_upload_pack_error(format!("git upload-pack body is too large: {error}"));
        }
    };
    let body = match decode_git_request_body(&headers, body, MAX_UPLOAD_PACK_BYTES) {
        Ok(body) => body,
        Err(error) => return git_upload_pack_error(error.message),
    };

    match git_upload_pack_response(
        &repo_path,
        &body,
        state.runtime_budgets.git_command_timeout(),
        permit,
    )
    .await
    {
        Ok(response) => response,
        Err(error) => git_upload_pack_error(error.message),
    }
}

pub(crate) fn decode_git_request_body(
    headers: &HeaderMap,
    body: Bytes,
    max_bytes: usize,
) -> Result<Vec<u8>, ApiError> {
    let mut encodings = headers.get_all(CONTENT_ENCODING).iter();
    let Some(encoding) = encodings.next() else {
        return Ok(body.to_vec());
    };
    if encodings.next().is_some() {
        return Err(ApiError::bad_request(
            "multiple Git content-encoding headers are unsupported",
        ));
    }

    let encoding = encoding
        .to_str()
        .map_err(|_| ApiError::bad_request("invalid Git content-encoding header"))?
        .trim();
    if encoding.is_empty() || encoding.eq_ignore_ascii_case("identity") {
        return Ok(body.to_vec());
    }
    if !encoding.eq_ignore_ascii_case("gzip") {
        return Err(ApiError::bad_request(format!(
            "unsupported Git content-encoding {encoding}"
        )));
    }

    let mut decoded = Vec::new();
    GzDecoder::new(body.as_ref())
        .take((max_bytes as u64).saturating_add(1))
        .read_to_end(&mut decoded)
        .map_err(|error| {
            ApiError::bad_request(format!("invalid gzip Git request body: {error}"))
        })?;
    if decoded.len() > max_bytes {
        return Err(ApiError::payload_too_large(
            "git request body is too large after decompression",
        ));
    }
    Ok(decoded)
}

pub(crate) async fn receive_pack_access(
    state: &AppState,
    headers: &HeaderMap,
    owner: &str,
    repo_name: &str,
) -> Result<ReceivePackAccess, ApiError> {
    let authorization = receive_pack_authorization(state, headers).await?;

    match authorization {
        ReceivePackAuthorization::ScopeToken { secret } => {
            let repo = find_repo_after_git_scope_token(state, owner, repo_name)?;
            let credential = if secret.starts_with(GIT_PUSH_TOKEN_PREFIX) {
                InitialPushCredential::GitPushToken { secret }
            } else {
                InitialPushCredential::FirstPushToken { secret }
            };

            if repo.is_waiting_for_first_push() {
                authorize_initial_push_for_repo(&repo, &credential)
                    .map_err(git_credential_error)?;
                return Ok(ReceivePackAccess::FirstPush { credential });
            }
            if repo.has_pending_import_review() {
                authorize_receive_pack_scope_token_for_repo(&repo, &credential)
                    .map_err(git_credential_error)?;
                return Err(ApiError::conflict(
                    "repo is waiting for publish and cannot receive another push",
                ));
            }
            match repo.record.publication_state {
                RepoPublicationState::Unpublished => Err(ApiError::conflict(
                    "repo is waiting for publish and cannot receive another push",
                )),
                RepoPublicationState::Published => match credential {
                    InitialPushCredential::GitPushToken { secret } => {
                        let author_id = authorize_git_write_token_for_repo(&repo, &secret)
                            .map_err(git_credential_error)?;
                        Ok(ReceivePackAccess::PublishedMember { author_id })
                    }
                    InitialPushCredential::FirstPushToken { .. } => Err(invalid_git_credentials()),
                },
            }
        }
        ReceivePackAuthorization::ScopeUser(user) => {
            let repo = find_repo(state, owner, repo_name)?;
            let principal = principal_for_user_id(&repo, &user.id);
            let access = repo.access_for_principal(&principal);
            if repo.is_waiting_for_first_push() {
                if access.actor != RepositoryActor::Owner {
                    return Err(ApiError::not_found(format!(
                        "repo {owner}/{repo_name} not found"
                    )));
                }
                return Err(ApiError::unauthorized(
                    "first-push token or Git push token required",
                ));
            }
            if repo.has_pending_import_review() {
                if access.actor != RepositoryActor::Owner {
                    return Err(ApiError::not_found(format!(
                        "repo {owner}/{repo_name} not found"
                    )));
                }
                return Err(ApiError::conflict(
                    "repo is waiting for publish and cannot receive another push",
                ));
            }
            if !access.can_push {
                return Err(ApiError::not_found(format!(
                    "repo {owner}/{repo_name} not found"
                )));
            }
            match repo.record.publication_state {
                RepoPublicationState::Unpublished => Err(ApiError::conflict(
                    "repo is waiting for publish and cannot receive another push",
                )),
                RepoPublicationState::Published => {
                    Ok(ReceivePackAccess::PublishedMember { author_id: user.id })
                }
            }
        }
    }
}

pub(crate) fn handle_git_receive_pack(
    state: &AppState,
    owner: &str,
    repo_name: &str,
    method: &str,
    body: Vec<u8>,
    content_type: Option<String>,
    access: ReceivePackAccess,
) -> Result<Response, ApiError> {
    let staging_repo = match &access {
        ReceivePackAccess::FirstPush { .. } => {
            ensure_first_push_receive_pack_staging_repo(state, owner, repo_name)?
        }
        ReceivePackAccess::PublishedMember { author_id, .. } => {
            ensure_published_receive_pack_staging_repo(state, owner, repo_name, author_id)?
        }
    };
    let remote_user = match &access {
        ReceivePackAccess::FirstPush { .. } => "first-push-token",
        ReceivePackAccess::PublishedMember { author_id, .. } => author_id.as_str(),
    };
    let receive_started_at = Instant::now();
    let refs_before_receive = if method == "POST" {
        match git_refs(&staging_repo) {
            Ok(refs) => Some(refs),
            Err(error) => {
                let _ = fs::remove_dir_all(&staging_repo);
                return Err(error);
            }
        }
    } else {
        None
    };
    let cgi = match git_http_backend(
        &staging_repo,
        method,
        if method == "GET" {
            "info/refs"
        } else {
            "git-receive-pack"
        },
        if method == "GET" {
            "service=git-receive-pack"
        } else {
            ""
        },
        body,
        content_type,
        remote_user,
    ) {
        Ok(cgi) => cgi,
        Err(error) => {
            let _ = fs::remove_dir_all(&staging_repo);
            return Err(error);
        }
    };
    let receive_elapsed = receive_started_at.elapsed();

    if method == "POST" && cgi.status.is_success() {
        let refs_after_receive = match git_refs(&staging_repo) {
            Ok(refs) => refs,
            Err(error) => {
                let _ = fs::remove_dir_all(&staging_repo);
                return Err(error);
            }
        };
        if refs_before_receive.as_ref() == Some(&refs_after_receive) {
            tracing::debug!(
                owner,
                repo = repo_name,
                receive_ms = receive_elapsed.as_millis(),
                "git receive-pack left refs unchanged"
            );
            let _ = fs::remove_dir_all(&staging_repo);
            return Ok(cgi.into_response());
        }

        match access {
            ReceivePackAccess::FirstPush { credential } => {
                let import_started_at = Instant::now();
                let import = match pending_import_from_staging_repo(
                    state,
                    owner,
                    repo_name,
                    &staging_repo,
                ) {
                    Ok(import) => import,
                    Err(error) => {
                        let _ = fs::remove_dir_all(&staging_repo);
                        return Err(error);
                    }
                };
                let file_count = import.files.len();
                let uploaded_blobs = import
                    .files
                    .iter()
                    .map(|file| file.blob.clone())
                    .chain(std::iter::once(import.git_snapshot.clone()))
                    .collect::<Vec<_>>();
                let change_version =
                    match persist_pending_import(state, owner, repo_name, &credential, import) {
                        Ok(change_version) => change_version,
                        Err(error) => {
                            crate::state::best_effort_cleanup_rollback_source_blobs(
                                state,
                                &uploaded_blobs,
                            );
                            let _ = fs::remove_dir_all(&staging_repo);
                            return Err(error);
                        }
                    };
                state.publish_repo_change(
                    &crate::domain::store::repo_id(owner, repo_name),
                    change_version,
                    "first-push-received",
                );
                tracing::info!(
                    owner,
                    repo = repo_name,
                    receive_ms = receive_elapsed.as_millis(),
                    import_ms = import_started_at.elapsed().as_millis(),
                    file_count,
                    "git receive-pack pending import persisted"
                );
            }
            ReceivePackAccess::PublishedMember { author_id } => {
                let import_started_at = Instant::now();
                let update = match receive_pack_update_from_staging_repo(
                    state,
                    owner,
                    repo_name,
                    &staging_repo,
                    &author_id,
                ) {
                    Ok(update) => update,
                    Err(error) => {
                        let _ = fs::remove_dir_all(&staging_repo);
                        return Err(error);
                    }
                };
                let change_count = update.changes.len();
                let uploaded_blobs = update
                    .uploaded_blobs
                    .iter()
                    .cloned()
                    .chain(std::iter::once(update.git_snapshot.clone()))
                    .collect::<Vec<_>>();
                if let Err(error) = persist_receive_pack_update_and_promote(
                    state, owner, repo_name, update, &author_id,
                ) {
                    crate::state::best_effort_cleanup_rollback_source_blobs(state, &uploaded_blobs);
                    let _ = fs::remove_dir_all(&staging_repo);
                    return Err(error);
                }
                let repo = find_repo(state, owner, repo_name)?;
                state.publish_repo_change(
                    &repo.record.id,
                    repo.record.change_version,
                    "push-received",
                );
                tracing::info!(
                    owner,
                    repo = repo_name,
                    receive_ms = receive_elapsed.as_millis(),
                    import_ms = import_started_at.elapsed().as_millis(),
                    change_count,
                    "git receive-pack published update persisted"
                );
            }
        }
    }

    let _ = fs::remove_dir_all(&staging_repo);
    Ok(cgi.into_response())
}
