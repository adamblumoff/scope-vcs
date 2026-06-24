use crate::domain::{
    policy::ScopePath,
    repo_actions::ensure_pending_publish,
    store::{
        RepoPublicationState, RepoRole, SourceBlob, StagedFileChangeKind, StagedRepoUpdate,
        StoredRepository,
    },
};
use crate::{
    auth::shoo::{
        ensure_user_for_identity, http_identity, principal_for_repo, principal_for_user_id,
    },
    error::ApiError,
    http::responses::*,
    object_store::{ObjectStore, source_blob_text},
    state::AppState,
    state::{
        best_effort_drain_pending_source_blob_deletions, ensure_owner, ensure_repo_read, find_repo,
    },
};
use axum::{
    Json,
    extract::{Path, Query, State},
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
        line_diff: pending_import_line_diff(&repo),
        files: pending_import_files(&repo, &principal)?,
    }))
}

pub(crate) async fn get_review_file_diff(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((owner, repo_name)): Path<(String, String)>,
    Query(input): Query<ReviewFileDiffRequest>,
) -> Result<Json<ReviewFileDiffResponse>, ApiError> {
    let repo = find_repo(&state, &owner, &repo_name)?;
    let identity = http_identity(&state, &headers).await?;
    let principal = principal_for_repo(&state, &repo, identity.as_ref())?;
    ensure_repo_read(&state, &repo, &principal)?;
    ensure_owner(&state, &repo, &principal)?;

    let path = pending_scope_path(&input.path)?;
    Ok(Json(review_file_diff_response(
        state.object_store.as_ref(),
        &repo,
        &path,
    )?))
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

    let staged = repo
        .staged_update
        .as_ref()
        .map(staged_update_response_with_diff);

    Ok(Json(staged))
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
    if input.paths.is_empty() {
        return Err(ApiError::bad_request("at least one file path is required"));
    }
    let paths = input
        .paths
        .iter()
        .map(|path| pending_scope_path(path))
        .collect::<Result<Vec<_>, _>>()?;

    let updated = state.metadata.update_staged_file_visibility(
        &owner,
        &repo_name,
        &principal.id,
        paths,
        input.visibility,
    )?;

    Ok(Json(staged_update_response_with_diff(&updated)))
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
    let applied = state
        .metadata
        .apply_staged_update(&owner, &repo_name, &principal.id)?;
    best_effort_drain_pending_source_blob_deletions(&state);

    Ok(Json(staged_update_response_with_diff(&applied)))
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
    let rejected = state
        .metadata
        .reject_staged_update(&owner, &repo_name, &principal.id)?;
    let response = staged_update_response_with_diff(&rejected);
    best_effort_drain_pending_source_blob_deletions(&state);

    Ok(Json(response))
}

fn review_file_diff_response(
    store: &dyn ObjectStore,
    repo: &StoredRepository,
    path: &ScopePath,
) -> Result<ReviewFileDiffResponse, ApiError> {
    if repo.record.publication_state == RepoPublicationState::PendingPublish {
        return pending_import_file_diff_response(store, repo, path);
    }

    let staged = repo
        .staged_update
        .as_ref()
        .ok_or_else(|| ApiError::bad_request("repo has no pending review"))?;
    staged_file_diff_response(store, staged, path)
}

fn pending_import_file_diff_response(
    store: &dyn ObjectStore,
    repo: &StoredRepository,
    path: &ScopePath,
) -> Result<ReviewFileDiffResponse, ApiError> {
    let pending = repo
        .pending_import
        .as_ref()
        .ok_or_else(|| ApiError::bad_request("repo has no pending import"))?;
    let selected = pending.files.iter().find_map(|file| {
        let file_path = pending_scope_path(&file.path).ok()?;
        (file_path.as_str() == path.as_str()).then_some((file, file_path))
    });
    let Some((file, file_path)) = selected else {
        return Err(ApiError::not_found(format!(
            "file {} not found",
            path.as_str()
        )));
    };

    Ok(ReviewFileDiffResponse {
        path: file_path.as_str().to_string(),
        kind: StagedFileChangeKind::Added,
        old_content: None,
        new_content: Some(source_blob_text(store, &file.blob)?),
    })
}

fn staged_file_diff_response(
    store: &dyn ObjectStore,
    staged: &StagedRepoUpdate,
    path: &ScopePath,
) -> Result<ReviewFileDiffResponse, ApiError> {
    let change = staged
        .changes
        .iter()
        .find(|change| change.path.as_str() == path.as_str())
        .ok_or_else(|| ApiError::not_found(format!("file {} not found", path.as_str())))?;

    Ok(ReviewFileDiffResponse {
        path: change.path.as_str().to_string(),
        kind: change.kind,
        old_content: source_blob_text_opt(store, change.old_content.as_ref())?,
        new_content: source_blob_text_opt(store, change.new_content.as_ref())?,
    })
}

fn source_blob_text_opt(
    store: &dyn ObjectStore,
    blob: Option<&SourceBlob>,
) -> Result<Option<String>, ApiError> {
    blob.map(|blob| source_blob_text(store, blob)).transpose()
}

fn pending_import_line_diff(repo: &StoredRepository) -> ReviewLineDiffResponse {
    let mut line_diff = ReviewLineDiffResponse {
        additions: 0,
        deletions: 0,
    };
    if let Some(pending) = &repo.pending_import {
        for file in &pending.files {
            line_diff.additions += file.blob.line_count;
        }
    }
    line_diff
}

fn staged_update_response_with_diff(update: &StagedRepoUpdate) -> StagedUpdateResponse {
    staged_update_response(update, staged_update_line_diff(update))
}

fn staged_update_line_diff(update: &StagedRepoUpdate) -> ReviewLineDiffResponse {
    let mut line_diff = ReviewLineDiffResponse {
        additions: 0,
        deletions: 0,
    };

    for change in &update.changes {
        line_diff.additions += change.line_diff.additions;
        line_diff.deletions += change.line_diff.deletions;
    }

    line_diff
}
