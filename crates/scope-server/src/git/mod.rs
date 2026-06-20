pub(crate) mod import;
pub(crate) mod storage;
pub(crate) mod upload;

use crate::{
    auth::{
        shoo::{ShooIdentity, ensure_user_for_identity, principal_for_user_id, require_identity},
        tokens::{first_push_token_hash, git_push_token_hash},
    },
    config::*,
    error::ApiError,
    git::{
        import::{
            pending_import_from_staging_repo, persist_pending_import, persist_receive_pack_update,
            receive_pack_update_from_staging_repo,
        },
        storage::*,
        upload::*,
    },
    http::responses::first_push_token_status_at,
    persistence::unix_now,
    state::AppState,
    state::{find_repo, role_for_principal},
};
use axum::{
    Json,
    body::to_bytes,
    extract::{Path, Query, Request, State},
    http::{
        HeaderMap, HeaderValue, StatusCode,
        header::{AUTHORIZATION, CONTENT_TYPE, WWW_AUTHENTICATE},
    },
    response::{IntoResponse, Response},
};
use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64};
use scope_store::{FirstPushTokenStatus, RepoPublicationState, RepoRole, StoredRepository};
use serde::Deserialize;
use std::fs;

#[derive(Debug, Deserialize)]
pub(crate) struct GitInfoRefsQuery {
    pub(crate) service: Option<String>,
}

#[derive(Debug)]
pub(crate) enum ReceivePackAccess {
    FirstPush { credential: InitialPushCredential },
    PublishedOwner { author_id: String },
}

#[derive(Clone, Debug)]
pub(crate) enum InitialPushCredential {
    FirstPushToken { secret: String },
    GitPushToken { secret: String },
}

#[derive(Clone, Debug)]
pub(crate) enum ReceivePackAuthorization {
    ScopeToken { secret: String },
    ShooIdentity(ShooIdentity),
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
                        let author_id = authorize_git_push_token_for_repo(&repo, &secret)
                            .map_err(git_credential_error)?;
                        Ok(ReceivePackAccess::PublishedOwner { author_id })
                    }
                    InitialPushCredential::FirstPushToken { .. } => Err(invalid_git_credentials()),
                },
            }
        }
        ReceivePackAuthorization::ShooIdentity(identity) => {
            let repo = find_repo(state, owner, repo_name)?;
            let user = ensure_user_for_identity(state, &identity)?;
            let principal = principal_for_user_id(&repo, &user.id);
            if role_for_principal(state, &repo, &principal)? != Some(RepoRole::Owner) {
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
                    Ok(ReceivePackAccess::PublishedOwner { author_id: user.id })
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
        ReceivePackAccess::PublishedOwner { author_id } => {
            ensure_published_receive_pack_staging_repo(state, owner, repo_name, author_id)?
        }
    };
    let remote_user = match &access {
        ReceivePackAccess::FirstPush { .. } => "first-push-token",
        ReceivePackAccess::PublishedOwner { author_id } => author_id.as_str(),
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
                let import = match pending_import_from_staging_repo(&staging_repo) {
                    Ok(import) => import,
                    Err(error) => {
                        let _ = fs::remove_dir_all(&staging_repo);
                        return Err(error);
                    }
                };
                if let Err(error) =
                    persist_pending_import(state, owner, repo_name, &credential, import)
                {
                    let _ = fs::remove_dir_all(&staging_repo);
                    return Err(error);
                }
                if let Err(error) =
                    replace_git_repo(&staging_repo, &owner_git_repo_path(state, owner, repo_name))
                {
                    let _ = fs::remove_dir_all(&staging_repo);
                    return Err(error);
                }
            }
            ReceivePackAccess::PublishedOwner { author_id } => {
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
                let persisted = match persist_receive_pack_update(state, owner, repo_name, update) {
                    Ok(persisted) => persisted,
                    Err(error) => {
                        let _ = fs::remove_dir_all(&staging_repo);
                        return Err(error);
                    }
                };
                let target_repo = match persisted {
                    PersistedReceivePackUpdate::Staged => {
                        staged_git_repo_path(state, owner, repo_name)
                    }
                    PersistedReceivePackUpdate::Applied => {
                        owner_git_repo_path(state, owner, repo_name)
                    }
                };
                if let Err(error) = replace_git_repo(&staging_repo, &target_repo) {
                    let _ = fs::remove_dir_all(&staging_repo);
                    return Err(error);
                }
            }
        }
    }

    let _ = fs::remove_dir_all(&staging_repo);
    Ok(cgi.into_response())
}

#[cfg(test)]
pub(crate) fn first_push_token_from_headers(headers: &HeaderMap) -> Result<String, ApiError> {
    let Some(value) = headers.get(AUTHORIZATION) else {
        return Err(ApiError::unauthorized("first-push token required"));
    };
    let value = value
        .to_str()
        .map_err(|_| ApiError::unauthorized("invalid authorization header"))?;

    if let Some(token) = value.strip_prefix("Bearer ") {
        let token = token.trim();
        if token.is_empty() {
            return Err(ApiError::unauthorized("empty first-push token"));
        }
        return Ok(token.to_string());
    }

    if let Some(encoded) = value.strip_prefix("Basic ") {
        let token = basic_auth_secret(encoded)?;
        if token.is_empty() {
            return Err(ApiError::unauthorized("empty first-push token"));
        }
        return Ok(token);
    }

    Err(ApiError::unauthorized(
        "expected Authorization: Basic or Bearer first-push token",
    ))
}

pub(crate) async fn receive_pack_authorization(
    state: &AppState,
    headers: &HeaderMap,
) -> Result<ReceivePackAuthorization, ApiError> {
    let Some(value) = headers.get(AUTHORIZATION) else {
        return Err(git_receive_pack_auth_required());
    };
    let value = value
        .to_str()
        .map_err(|_| ApiError::unauthorized("invalid authorization header"))?;

    if let Some(encoded) = value.strip_prefix("Basic ") {
        let secret = basic_auth_secret(encoded)?;
        if secret.is_empty() {
            return Err(ApiError::unauthorized("empty Git push token"));
        }
        return Ok(ReceivePackAuthorization::ScopeToken { secret });
    }

    if let Some(token) = value.strip_prefix("Bearer ") {
        let token = token.trim();
        if token.is_empty() {
            return Err(ApiError::unauthorized("empty bearer token"));
        }
        if token.starts_with(FIRST_PUSH_TOKEN_PREFIX) || token.starts_with(GIT_PUSH_TOKEN_PREFIX) {
            return Ok(ReceivePackAuthorization::ScopeToken {
                secret: token.to_string(),
            });
        }

        return require_identity(state, headers)
            .await
            .map(ReceivePackAuthorization::ShooIdentity);
    }

    Err(ApiError::unauthorized(
        "expected Authorization: Basic or Bearer Git credentials",
    ))
}

pub(crate) fn git_receive_pack_auth_required() -> ApiError {
    ApiError::unauthorized("Git push credentials required")
}

pub(crate) fn git_push_token_from_headers(headers: &HeaderMap) -> Result<Option<String>, ApiError> {
    let Some(value) = headers.get(AUTHORIZATION) else {
        return Ok(None);
    };
    let value = value
        .to_str()
        .map_err(|_| ApiError::unauthorized("invalid authorization header"))?;

    if let Some(token) = value.strip_prefix("Bearer ") {
        let token = token.trim();
        if token.is_empty() {
            return Err(ApiError::unauthorized("empty bearer token"));
        }
        return if token.starts_with(GIT_PUSH_TOKEN_PREFIX) {
            Ok(Some(token.to_string()))
        } else {
            Ok(None)
        };
    }

    if let Some(encoded) = value.strip_prefix("Basic ") {
        let token = basic_auth_secret(encoded)?;
        if token.is_empty() {
            return Err(ApiError::unauthorized("empty Git push token"));
        }
        return Ok(Some(token));
    }

    Err(ApiError::unauthorized(
        "expected Authorization: Basic Git push token or Bearer token",
    ))
}

pub(crate) fn basic_auth_secret(encoded: &str) -> Result<String, ApiError> {
    let decoded = BASE64
        .decode(encoded.trim())
        .map_err(|_| ApiError::unauthorized("invalid basic authorization"))?;
    let decoded = String::from_utf8(decoded)
        .map_err(|_| ApiError::unauthorized("invalid basic authorization"))?;
    let (username, password) = decoded.split_once(':').unwrap_or((decoded.as_str(), ""));
    let token = if password.is_empty() {
        username.trim()
    } else {
        password.trim()
    };

    Ok(token.to_string())
}

#[cfg(test)]
pub(crate) fn authorize_first_push(
    state: &AppState,
    owner: &str,
    repo_name: &str,
    token_secret: &str,
) -> Result<(), ApiError> {
    authorize_initial_push(
        state,
        owner,
        repo_name,
        &InitialPushCredential::FirstPushToken {
            secret: token_secret.to_string(),
        },
    )
}

#[cfg(test)]
pub(crate) fn authorize_initial_push(
    state: &AppState,
    owner: &str,
    repo_name: &str,
    credential: &InitialPushCredential,
) -> Result<(), ApiError> {
    let repo = find_repo(state, owner, repo_name)?;
    authorize_initial_push_for_repo(&repo, credential)
}

pub(crate) fn authorize_initial_push_for_repo(
    repo: &StoredRepository,
    credential: &InitialPushCredential,
) -> Result<(), ApiError> {
    if repo.record.publication_state != RepoPublicationState::PendingFirstPush {
        return Err(ApiError::conflict(
            "repo is not waiting for an initial Git push",
        ));
    }
    if repo.pending_import.is_some() {
        return Err(ApiError::conflict("repo already has a pending import"));
    }

    match credential {
        InitialPushCredential::FirstPushToken { secret } => {
            authorize_first_push_token_for_repo(repo, secret)
        }
        InitialPushCredential::GitPushToken { secret } => {
            authorize_git_push_token_for_repo(repo, secret).map(|_| ())
        }
    }
}

pub(crate) fn authorize_receive_pack_scope_token_for_repo(
    repo: &StoredRepository,
    credential: &InitialPushCredential,
) -> Result<(), ApiError> {
    match credential {
        InitialPushCredential::FirstPushToken { secret } => {
            authorize_first_push_token_for_repo(repo, secret)
        }
        InitialPushCredential::GitPushToken { secret } => {
            authorize_git_push_token_for_repo(repo, secret).map(|_| ())
        }
    }
}

pub(crate) fn find_repo_after_git_scope_token(
    state: &AppState,
    owner: &str,
    repo_name: &str,
) -> Result<StoredRepository, ApiError> {
    match find_repo(state, owner, repo_name) {
        Ok(repo) => Ok(repo),
        Err(error) if error.status == StatusCode::NOT_FOUND => Err(invalid_git_credentials()),
        Err(error) => Err(error),
    }
}

pub(crate) fn invalid_git_credentials() -> ApiError {
    ApiError::unauthorized("invalid Git credentials")
}

pub(crate) fn git_credential_error(error: ApiError) -> ApiError {
    if error.status == StatusCode::UNAUTHORIZED {
        invalid_git_credentials()
    } else {
        error
    }
}

pub(crate) fn authorize_first_push_token_for_repo(
    repo: &StoredRepository,
    token_secret: &str,
) -> Result<(), ApiError> {
    let now = unix_now()?;
    let Some(token) = repo.first_push_token.as_ref() else {
        return Err(ApiError::unauthorized("first-push token is not configured"));
    };
    if token.owner_user_id != repo.record.owner_user_id {
        return Err(ApiError::forbidden(
            "first-push token owner does not match repo owner",
        ));
    }
    if first_push_token_status_at(token, now) != FirstPushTokenStatus::Active {
        return Err(ApiError::unauthorized(
            "first-push token is expired or used",
        ));
    }
    if token.token_hash != first_push_token_hash(token_secret) {
        return Err(ApiError::unauthorized("invalid first-push token"));
    }

    Ok(())
}

pub(crate) fn authorize_git_push_token_for_repo(
    repo: &StoredRepository,
    secret: &str,
) -> Result<String, ApiError> {
    let Some(token) = repo.git_push_token.as_ref() else {
        return Err(ApiError::unauthorized("Git push token is not configured"));
    };
    if token.owner_user_id != repo.record.owner_user_id {
        return Err(ApiError::forbidden(
            "Git push token owner does not match repo owner",
        ));
    }
    if token.token_hash != git_push_token_hash(secret) {
        return Err(ApiError::unauthorized("invalid Git push token"));
    }

    Ok(token.owner_user_id.clone())
}
