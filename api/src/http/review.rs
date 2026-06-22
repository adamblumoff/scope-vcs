use crate::domain::store::RepoRole;
use crate::{
    auth::shoo::{
        ensure_user_for_identity, http_identity, principal_for_repo, principal_for_user_id,
    },
    error::ApiError,
    git::import::{apply_receive_pack_update, validate_staged_update_policy},
    http::responses::*,
    state::AppState,
    state::{
        best_effort_drain_pending_source_blob_deletions, ensure_owner, ensure_pending_publish,
        ensure_repo_read, find_repo, queue_source_blob_deletions,
    },
};
use axum::{
    Json,
    extract::{Path, State},
    http::HeaderMap,
};

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
    let user = identity
        .as_ref()
        .map(|identity| ensure_user_for_identity(&state, identity))
        .transpose()?;
    let repo = find_repo(&state, &owner, &repo_name)?;
    let principal = user
        .as_ref()
        .map(|user| principal_for_user_id(&repo, &user.id))
        .unwrap_or_else(crate::domain::policy::Principal::public);
    ensure_repo_read(&state, &repo, &principal)?;
    ensure_owner(&state, &repo, &principal)?;
    ensure_pending_publish(&repo)?;

    let user_id = user
        .as_ref()
        .map(|user| user.id.clone())
        .ok_or_else(|| ApiError::forbidden("owner role required"))?;
    let updated = state
        .metadata
        .publish_pending_import(&owner, &repo_name, &user_id)?;

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
    let repo_id = crate::domain::store::repo_id(&owner, &repo_name);
    if input.paths.is_empty() {
        return Err(ApiError::bad_request("at least one file path is required"));
    }
    let paths = input
        .paths
        .iter()
        .map(|path| pending_scope_path(path))
        .collect::<Result<Vec<_>, _>>()?;

    let updated = state.metadata.update(move |catalog| {
        let repo = catalog
            .repositories
            .get_mut(&repo_id)
            .ok_or_else(|| ApiError::not_found(format!("repo {owner}/{repo_name} not found")))?;
        let mut staged_update = repo
            .staged_update
            .clone()
            .ok_or_else(|| ApiError::not_found("no staged update pending"))?;
        for path in &paths {
            let file = staged_update
                .changes
                .iter_mut()
                .find(|change| change.path == *path)
                .ok_or_else(|| ApiError::not_found(format!("staged file {} not found", path)))?;
            file.visibility = input.visibility;
        }
        validate_staged_update_policy(repo, &staged_update)?;
        let updated = staged_update_response(&staged_update);
        repo.staged_update = Some(staged_update);

        Ok(updated)
    })?;

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
    let applied = apply_staged_update_in_catalog(&state, &owner, &repo_name)?;

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
    let rejected = reject_staged_update_in_catalog(&state, &owner, &repo_name)?;

    Ok(Json(rejected))
}

pub(crate) fn apply_staged_update_in_catalog(
    state: &AppState,
    owner: &str,
    repo_name: &str,
) -> Result<StagedUpdateResponse, ApiError> {
    let repo_id = crate::domain::store::repo_id(owner, repo_name);
    let owner = owner.to_string();
    let repo_name = repo_name.to_string();
    let response = state.metadata.update(move |catalog| {
        let repo = catalog
            .repositories
            .get_mut(&repo_id)
            .ok_or_else(|| ApiError::not_found(format!("repo {owner}/{repo_name} not found")))?;
        let old_snapshot = repo.git_snapshot.clone();
        let staged_update = repo
            .staged_update
            .take()
            .ok_or_else(|| ApiError::not_found("no staged update pending"))?;
        let response = staged_update_response(&staged_update);
        apply_receive_pack_update(repo, staged_update)?;
        queue_source_blob_deletions(catalog, old_snapshot);
        Ok(response)
    })?;
    best_effort_drain_pending_source_blob_deletions(state);
    Ok(response)
}

pub(crate) fn reject_staged_update_in_catalog(
    state: &AppState,
    owner: &str,
    repo_name: &str,
) -> Result<StagedUpdateResponse, ApiError> {
    let repo_id = crate::domain::store::repo_id(owner, repo_name);
    let owner = owner.to_string();
    let repo_name = repo_name.to_string();
    let response = state.metadata.update(move |catalog| {
        let repo = catalog
            .repositories
            .get_mut(&repo_id)
            .ok_or_else(|| ApiError::not_found(format!("repo {owner}/{repo_name} not found")))?;
        let staged_update = repo
            .staged_update
            .take()
            .ok_or_else(|| ApiError::not_found("no staged update pending"))?;
        let response = staged_update_response(&staged_update);
        let rejected_blobs = std::iter::once(staged_update.git_snapshot.clone())
            .chain(
                staged_update
                    .changes
                    .iter()
                    .filter_map(|change| change.new_content.clone()),
            )
            .collect::<Vec<_>>();
        queue_source_blob_deletions(catalog, rejected_blobs);
        Ok(response)
    })?;
    best_effort_drain_pending_source_blob_deletions(state);
    Ok(response)
}
