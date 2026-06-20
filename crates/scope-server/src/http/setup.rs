use crate::{
    auth::{
        shoo::{ensure_user_for_identity, require_identity},
        tokens::{
            ensure_owner_setup_access, ensure_owner_setup_access_in_catalog,
            generate_first_push_token, generate_git_push_token,
        },
    },
    error::ApiError,
    http::responses::{RepoSetupResponse, repo_setup_response},
    persistence::{lock_catalog, persist_catalog, unix_now},
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
    ensure_owner_setup_access(&state, &repo, &user.id)?;
    let catalog = lock_catalog(&state)?;
    Ok(Json(repo_setup_response(
        &catalog,
        &repo,
        &user.id,
        unix_now()?,
        None,
        None,
    )?))
}

pub(crate) async fn regenerate_first_push_token(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((owner, repo_name)): Path<(String, String)>,
) -> Result<Json<RepoSetupResponse>, ApiError> {
    let identity = require_identity(&state, &headers).await?;
    let user = ensure_user_for_identity(&state, &identity)?;
    let repo_id = scope_store::repo_id(&owner, &repo_name);
    let now = unix_now()?;

    let setup = {
        let mut catalog = lock_catalog(&state)?;
        let mut staged = catalog.clone();
        let repo = staged
            .repositories
            .get(&repo_id)
            .ok_or_else(|| ApiError::not_found(format!("repo {owner}/{repo_name} not found")))?;
        ensure_owner_setup_access_in_catalog(&staged, repo, &user.id)?;

        let (secret, token) = generate_first_push_token(&user.id)?;
        {
            let repo = staged
                .repositories
                .get_mut(&repo_id)
                .expect("repo was already checked");
            repo.first_push_token = Some(token);
            if repo.git_push_token.is_none() {
                let (_, push_token) = generate_git_push_token(&user.id)?;
                repo.git_push_token = Some(push_token);
            }
        }
        let repo = staged
            .repositories
            .get(&repo_id)
            .expect("repo was already checked");
        let setup = repo_setup_response(&staged, repo, &user.id, now, Some(secret), None)?;

        persist_catalog(&state, &staged)?;
        *catalog = staged;
        setup
    };

    Ok(Json(setup))
}
