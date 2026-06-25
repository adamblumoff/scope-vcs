use crate::domain::store::{
    FirstPushTokenStatus, RepoPublicationState, RepoRole, StoredRepository,
};
use crate::{
    auth::{
        clerk::{ClerkIdentity, require_identity},
        tokens::{first_push_token_hash, git_clone_token_hash, git_push_token_hash},
    },
    config::{FIRST_PUSH_TOKEN_PREFIX, GIT_PUSH_TOKEN_PREFIX},
    error::ApiError,
    persistence::unix_now,
    state::{AppState, find_repo},
};
use axum::http::{HeaderMap, StatusCode, header::AUTHORIZATION};
use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64};

#[derive(Clone, Debug)]
pub(crate) enum InitialPushCredential {
    FirstPushToken { secret: String },
    GitPushToken { secret: String },
}

#[derive(Clone, Debug)]
pub(crate) enum ReceivePackAuthorization {
    ScopeToken { secret: String },
    ClerkIdentity(ClerkIdentity),
}

#[derive(Clone, Debug)]
pub(crate) enum GitReadAuthorization {
    ScopeToken { secret: String },
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
            .map(ReceivePackAuthorization::ClerkIdentity);
    }

    Err(ApiError::unauthorized(
        "expected Authorization: Basic or Bearer Git credentials",
    ))
}

pub(crate) fn git_receive_pack_auth_required() -> ApiError {
    ApiError::unauthorized("Git push credentials required")
}

pub(crate) fn git_read_authorization_from_headers(
    headers: &HeaderMap,
) -> Result<Option<GitReadAuthorization>, ApiError> {
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
        if token.starts_with(GIT_PUSH_TOKEN_PREFIX) {
            return Ok(Some(GitReadAuthorization::ScopeToken {
                secret: token.to_string(),
            }));
        }
        return Ok(None);
    }

    if let Some(encoded) = value.strip_prefix("Basic ") {
        let token = basic_auth_secret(encoded)?;
        if token.is_empty() {
            return Err(ApiError::unauthorized("empty Git token"));
        }
        return Ok(Some(GitReadAuthorization::ScopeToken { secret: token }));
    }

    Err(ApiError::unauthorized(
        "expected Authorization: Basic Git token or Bearer token",
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
    if token.status_at(now) != FirstPushTokenStatus::Active {
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

pub(crate) fn authorize_git_scope_token_for_repo(
    repo: &StoredRepository,
    secret: &str,
) -> Result<String, ApiError> {
    if let Some(token) = repo.git_push_token.as_ref()
        && token.token_hash == git_push_token_hash(secret)
    {
        if token.owner_user_id != repo.record.owner_user_id {
            return Err(ApiError::forbidden(
                "Git push token owner does not match repo owner",
            ));
        }
        return Ok(token.owner_user_id.clone());
    }

    authorize_git_member_token_for_repo(repo, secret, None)
}

pub(crate) fn authorize_git_write_token_for_repo(
    repo: &StoredRepository,
    secret: &str,
) -> Result<String, ApiError> {
    if let Some(token) = repo.git_push_token.as_ref()
        && token.token_hash == git_push_token_hash(secret)
    {
        if token.owner_user_id != repo.record.owner_user_id {
            return Err(ApiError::forbidden(
                "Git push token owner does not match repo owner",
            ));
        }
        return Ok(token.owner_user_id.clone());
    }

    authorize_git_member_token_for_repo(repo, secret, Some(RepoRole::Writer))
}

pub(crate) fn authorize_git_member_token_for_repo(
    repo: &StoredRepository,
    secret: &str,
    minimum_role: Option<RepoRole>,
) -> Result<String, ApiError> {
    let hash = git_clone_token_hash(secret);
    let Some(token) = repo
        .git_clone_tokens
        .iter()
        .find(|token| token.token_hash == hash)
    else {
        return Err(ApiError::unauthorized("invalid Git credentials"));
    };

    let role = if token.user_id == repo.record.owner_user_id {
        RepoRole::Owner
    } else {
        let Some(role) = repo
            .memberships
            .iter()
            .find(|membership| membership.user_id == token.user_id)
            .map(|membership| membership.role)
        else {
            return Err(ApiError::unauthorized("invalid Git credentials"));
        };
        role
    };

    if minimum_role.is_some_and(|minimum| role < minimum) {
        return Err(ApiError::unauthorized("invalid Git credentials"));
    }

    Ok(token.user_id.clone())
}
