mod credentials;
pub(crate) mod import;
pub(crate) mod storage;
pub(crate) mod upload;

pub(crate) use credentials::*;

use crate::domain::store::{RepoPublicationState, RepoRole};
use crate::{
    auth::clerk::{ensure_user_for_identity, principal_for_user_id},
    config::*,
    error::ApiError,
    git::{
        import::{
            pending_import_from_staging_repo, persist_pending_import,
            persist_receive_pack_update_and_promote, receive_pack_update_from_staging_repo,
        },
        storage::*,
        upload::*,
    },
    state::AppState,
    state::{find_repo, role_for_principal},
};
use axum::{
    Json,
    body::to_bytes,
    extract::{Path, Query, Request, State},
    http::{
        HeaderMap, HeaderValue, StatusCode,
        header::{CONTENT_TYPE, WWW_AUTHENTICATE},
    },
    response::{IntoResponse, Response},
};
use serde::Deserialize;
use std::fs;

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
                match handle_git_receive_pack(&state, &org, &repo, "GET", Vec::new(), None, access)
                {
                    Ok(response) => response,
                    Err(error) => git_error_response(error),
                }
            }
            Err(error) => git_error_response(error),
        },
        Some(GIT_UPLOAD_PACK) => {
            match git_upload_pack_repo_for_request(&state, &headers, &org, &repo).await {
                Ok(repo_path) => git_upload_pack_advertisement(&repo_path),
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

    let content_type = headers
        .get(CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .map(str::to_string);
    let body = match to_bytes(request.into_body(), MAX_RECEIVE_PACK_BYTES).await {
        Ok(body) => body,
        Err(error) => {
            return (
                StatusCode::PAYLOAD_TOO_LARGE,
                Json(serde_json::json!({
                    "error": format!("git receive-pack body is too large: {error}")
                })),
            )
                .into_response();
        }
    };

    match handle_git_receive_pack(
        &state,
        &org,
        &repo,
        "POST",
        body.to_vec(),
        content_type,
        access,
    ) {
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

    match git_upload_pack_response(&repo_path, &body).await {
        Ok(response) => response,
        Err(error) => git_upload_pack_error(error.message),
    }
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

            match repo.record.publication_state {
                RepoPublicationState::PendingFirstPush => {
                    authorize_initial_push_for_repo(&repo, &credential)
                        .map_err(git_credential_error)?;
                    Ok(ReceivePackAccess::FirstPush { credential })
                }
                RepoPublicationState::PendingPublish => {
                    authorize_receive_pack_scope_token_for_repo(&repo, &credential)
                        .map_err(git_credential_error)?;
                    Err(ApiError::conflict(
                        "repo is waiting for publish and cannot receive another push",
                    ))
                }
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
        ReceivePackAuthorization::ClerkIdentity(identity) => {
            let repo = find_repo(state, owner, repo_name)?;
            let user = ensure_user_for_identity(state, &identity)?;
            let principal = principal_for_user_id(&repo, &user.id);
            if !role_for_principal(state, &repo, &principal)?
                .is_some_and(|role| role >= RepoRole::Writer)
            {
                return Err(ApiError::not_found(format!(
                    "repo {owner}/{repo_name} not found"
                )));
            }

            match repo.record.publication_state {
                RepoPublicationState::PendingFirstPush => Err(ApiError::unauthorized(
                    "first-push token or Git push token required",
                )),
                RepoPublicationState::PendingPublish => Err(ApiError::conflict(
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
        ReceivePackAccess::PublishedMember { author_id } => {
            ensure_published_receive_pack_staging_repo(state, owner, repo_name, author_id)?
        }
    };
    let remote_user = match &access {
        ReceivePackAccess::FirstPush { .. } => "first-push-token",
        ReceivePackAccess::PublishedMember { author_id } => author_id.as_str(),
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

    if method == "POST" && cgi.status.is_success() {
        match access {
            ReceivePackAccess::FirstPush { credential } => {
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
                let uploaded_blobs = import
                    .files
                    .iter()
                    .map(|file| file.blob.clone())
                    .chain(std::iter::once(import.git_snapshot.clone()))
                    .collect::<Vec<_>>();
                if let Err(error) =
                    persist_pending_import(state, owner, repo_name, &credential, import)
                {
                    crate::state::best_effort_cleanup_rollback_source_blobs(state, &uploaded_blobs);
                    let _ = fs::remove_dir_all(&staging_repo);
                    return Err(error);
                }
            }
            ReceivePackAccess::PublishedMember { author_id } => {
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
                let uploaded_blobs = update
                    .uploaded_blobs
                    .iter()
                    .cloned()
                    .chain(std::iter::once(update.git_snapshot.clone()))
                    .collect::<Vec<_>>();
                if let Err(error) =
                    persist_receive_pack_update_and_promote(state, owner, repo_name, update)
                {
                    crate::state::best_effort_cleanup_rollback_source_blobs(state, &uploaded_blobs);
                    let _ = fs::remove_dir_all(&staging_repo);
                    return Err(error);
                }
            }
        }
    }

    let _ = fs::remove_dir_all(&staging_repo);
    Ok(cgi.into_response())
}
