use crate::{
    auth::shoo::{http_identity, identity_user_id, principal_for_repo, principal_for_user_id},
    error::ApiError,
    git::{
        import::{apply_receive_pack_update, validate_staged_update_policy},
        storage::{
            owner_git_repo_path, remove_git_repo_and_then, replace_git_repo_and_then,
            staged_git_repo_path,
        },
    },
    http::responses::*,
    persistence::{lock_catalog, persist_catalog},
    state::AppState,
    state::{
        ensure_owner, ensure_pending_publish, ensure_repo_read, find_repo, promote_pending_import,
    },
};
use axum::{
    Json,
    extract::{Path, State},
    http::HeaderMap,
};
use scope_store::RepoRole;

pub(crate) async fn get_pending_import_review(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((owner, repo_name)): Path<(String, String)>,
) -> Result<Json<PendingImportReviewResponse>, ApiError> {
    let repo = find_repo(&state, &owner, &repo_name)?;
    let identity = http_identity(&state, &headers).await?;
    let principal = principal_for_repo(&state, &repo, identity.as_ref())?;
    ensure_repo_read(&state, &repo, &principal)?;
    ensure_owner(&state, &repo, &principal)?;
    ensure_pending_publish(&repo)?;

    Ok(Json(PendingImportReviewResponse {
        publication_state: repo.record.publication_state,
        default_visibility: repo.record.default_visibility,
        files: pending_import_files(&repo, &principal)?,
    }))
}

pub(crate) async fn publish_repo(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((owner, repo_name)): Path<(String, String)>,
) -> Result<Json<SessionRepo>, ApiError> {
    let identity = http_identity(&state, &headers).await?;
    let repo_id = scope_store::repo_id(&owner, &repo_name);
    let repo = find_repo(&state, &owner, &repo_name)?;
    let principal = principal_for_repo(&state, &repo, identity.as_ref())?;
    ensure_repo_read(&state, &repo, &principal)?;
    ensure_owner(&state, &repo, &principal)?;
    ensure_pending_publish(&repo)?;

    let updated = {
        let mut catalog = lock_catalog(&state)?;
        let mut staged = catalog.clone();
        let repo = staged
            .repositories
            .get(&repo_id)
            .ok_or_else(|| ApiError::not_found(format!("repo {owner}/{repo_name} not found")))?;
        let user_id = identity
            .as_ref()
            .map(identity_user_id)
            .ok_or_else(|| ApiError::forbidden("owner role required"))?;
        let principal = principal_for_user_id(repo, &user_id);
        if staged.role_for_principal(repo, &principal) != Some(RepoRole::Owner) {
            return Err(ApiError::forbidden("owner role required"));
        }
        ensure_pending_publish(repo)?;

        {
            let repo = staged
                .repositories
                .get_mut(&repo_id)
                .expect("repo was already checked");
            promote_pending_import(repo)?;
        }

        persist_catalog(&state, &staged)?;
        let updated = staged
            .repositories
            .get(&repo_id)
            .expect("repo was already checked")
            .record
            .clone();
        *catalog = staged;
        updated
    };

    Ok(Json(SessionRepo {
        id: updated.id,
        publication_state: updated.publication_state,
        role: Some(RepoRole::Owner),
    }))
}

pub(crate) async fn get_staged_update(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((owner, repo_name)): Path<(String, String)>,
) -> Result<Json<Option<StagedUpdateResponse>>, ApiError> {
    let repo = find_repo(&state, &owner, &repo_name)?;
    let identity = http_identity(&state, &headers).await?;
    let principal = principal_for_repo(&state, &repo, identity.as_ref())?;
    ensure_repo_read(&state, &repo, &principal)?;
    ensure_owner(&state, &repo, &principal)?;

    Ok(Json(
        repo.staged_update.as_ref().map(staged_update_response),
    ))
}

pub(crate) async fn update_staged_file_visibility(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((owner, repo_name)): Path<(String, String)>,
    Json(input): Json<UpdateStagedFileVisibilityRequest>,
) -> Result<Json<StagedUpdateResponse>, ApiError> {
    let repo = find_repo(&state, &owner, &repo_name)?;
    let identity = http_identity(&state, &headers).await?;
    let principal = principal_for_repo(&state, &repo, identity.as_ref())?;
    ensure_repo_read(&state, &repo, &principal)?;
    ensure_owner(&state, &repo, &principal)?;
    let repo_id = scope_store::repo_id(&owner, &repo_name);
    let path = pending_scope_path(&input.path)?;

    let updated = {
        let mut catalog = lock_catalog(&state)?;
        let mut staged = catalog.clone();
        let repo = staged
            .repositories
            .get_mut(&repo_id)
            .ok_or_else(|| ApiError::not_found(format!("repo {owner}/{repo_name} not found")))?;
        let mut staged_update = repo
            .staged_update
            .clone()
            .ok_or_else(|| ApiError::not_found("no staged update pending"))?;
        let file = staged_update
            .changes
            .iter_mut()
            .find(|change| change.path == path)
            .ok_or_else(|| ApiError::not_found(format!("staged file {} not found", path)))?;
        file.visibility = input.visibility;
        validate_staged_update_policy(repo, &staged_update)?;
        let updated = staged_update_response(&staged_update);
        repo.staged_update = Some(staged_update);

        persist_catalog(&state, &staged)?;
        *catalog = staged;
        updated
    };

    Ok(Json(updated))
}

pub(crate) async fn apply_staged_update(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((owner, repo_name)): Path<(String, String)>,
) -> Result<Json<StagedUpdateResponse>, ApiError> {
    let repo = find_repo(&state, &owner, &repo_name)?;
    let identity = http_identity(&state, &headers).await?;
    let principal = principal_for_repo(&state, &repo, identity.as_ref())?;
    ensure_repo_read(&state, &repo, &principal)?;
    ensure_owner(&state, &repo, &principal)?;
    let staged_repo = staged_git_repo_path(&state, &owner, &repo_name);
    let applied = if staged_repo.exists() {
        replace_git_repo_and_then(
            &staged_repo,
            &owner_git_repo_path(&state, &owner, &repo_name),
            || apply_staged_update_in_catalog(&state, &owner, &repo_name),
        )?
    } else {
        apply_staged_update_in_catalog(&state, &owner, &repo_name)?
    };

    Ok(Json(applied))
}

pub(crate) async fn reject_staged_update(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((owner, repo_name)): Path<(String, String)>,
) -> Result<Json<StagedUpdateResponse>, ApiError> {
    let repo = find_repo(&state, &owner, &repo_name)?;
    let identity = http_identity(&state, &headers).await?;
    let principal = principal_for_repo(&state, &repo, identity.as_ref())?;
    ensure_repo_read(&state, &repo, &principal)?;
    ensure_owner(&state, &repo, &principal)?;
    let staged_repo = staged_git_repo_path(&state, &owner, &repo_name);
    let rejected = if staged_repo.exists() {
        remove_git_repo_and_then(&staged_repo, || {
            reject_staged_update_in_catalog(&state, &owner, &repo_name)
        })?
    } else {
        reject_staged_update_in_catalog(&state, &owner, &repo_name)?
    };

    Ok(Json(rejected))
}

pub(crate) fn apply_staged_update_in_catalog(
    state: &AppState,
    owner: &str,
    repo_name: &str,
) -> Result<StagedUpdateResponse, ApiError> {
    let repo_id = scope_store::repo_id(owner, repo_name);
    let mut catalog = lock_catalog(state)?;
    let mut staged = catalog.clone();
    let repo = staged
        .repositories
        .get_mut(&repo_id)
        .ok_or_else(|| ApiError::not_found(format!("repo {owner}/{repo_name} not found")))?;
    let staged_update = repo
        .staged_update
        .take()
        .ok_or_else(|| ApiError::not_found("no staged update pending"))?;
    let response = staged_update_response(&staged_update);
    apply_receive_pack_update(repo, staged_update)?;

    persist_catalog(state, &staged)?;
    *catalog = staged;
    Ok(response)
}

pub(crate) fn reject_staged_update_in_catalog(
    state: &AppState,
    owner: &str,
    repo_name: &str,
) -> Result<StagedUpdateResponse, ApiError> {
    let repo_id = scope_store::repo_id(owner, repo_name);
    let mut catalog = lock_catalog(state)?;
    let mut staged = catalog.clone();
    let repo = staged
        .repositories
        .get_mut(&repo_id)
        .ok_or_else(|| ApiError::not_found(format!("repo {owner}/{repo_name} not found")))?;
    let staged_update = repo
        .staged_update
        .take()
        .ok_or_else(|| ApiError::not_found("no staged update pending"))?;
    let response = staged_update_response(&staged_update);

    persist_catalog(state, &staged)?;
    *catalog = staged;
    Ok(response)
}
