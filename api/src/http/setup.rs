use crate::{
    auth::{
        shoo::{ensure_user_for_identity, require_identity},
        tokens::{generate_first_push_token, generate_git_push_token},
    },
    error::ApiError,
    http::responses::{RepoSetupResponse, repo_setup_response},
    persistence::unix_now,
    state::{AppState, find_repo},
};
use axum::{
    Json,
    extract::{Path, State},
    http::HeaderMap,
};

pub(crate) async fn get_repo_setup(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((owner, repo_name)): Path<(String, String)>,
) -> Result<Json<RepoSetupResponse>, ApiError> {
    let identity = require_identity(&state, &headers).await?;
    let user = ensure_user_for_identity(&state, &identity)?;
    let repo = find_repo(&state, &owner, &repo_name)?;
    let now = unix_now()?;
    let setup = repo_setup_response(&repo, &user.id, now, None, None)?;
    Ok(Json(setup))
}

pub(crate) async fn regenerate_first_push_token(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((owner, repo_name)): Path<(String, String)>,
) -> Result<Json<RepoSetupResponse>, ApiError> {
    let identity = require_identity(&state, &headers).await?;
    let user = ensure_user_for_identity(&state, &identity)?;
    let now = unix_now()?;

    let (secret, token) = generate_first_push_token(&user.id)?;
    let (push_secret, push_token) = generate_git_push_token(&user.id)?;
    let repo = state
        .metadata
        .regenerate_repo_setup_tokens(&owner, &repo_name, &user.id, token, push_token)?;

    let setup = repo_setup_response(&repo, &user.id, now, Some(secret), Some(push_secret))?;

    Ok(Json(setup))
}
