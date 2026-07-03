use crate::domain::{
    policy::ScopePath,
    repo_actions::ensure_pending_publish,
    store::{SourceBlob, StagedFileChangeKind, StagedRepoUpdate, StoredRepository},
};
use crate::{
    auth::scope::{optional_scope_user, principal_for_scope_user},
    error::ApiError,
    http::file_diffs::{
        add_line_diff, review_file_diff_response_for_blobs, review_line_diff_for_blobs,
    },
    http::responses::*,
    object_store::{ObjectStore, source_blob_bytes},
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
    let user = optional_scope_user(&state, &headers).await?;
    let principal = principal_for_scope_user(&repo, user.as_ref());
    ensure_repo_read(&state, &repo, &principal)?;
    if repo.has_pending_import_review() {
        ensure_owner(&state, &repo, &principal)?;
    } else if !repo.access_for_principal(&principal).can_apply_changes
        && !repo
            .access_for_principal(&principal)
            .can_change_file_visibility
    {
        return Err(ApiError::forbidden("review permission required"));
    }
    // Without a pending import this route intentionally returns 400 for
    // staged-review users while unrelated readers still get 403 above.
    ensure_pending_publish(&repo)?;

    Ok(Json(PendingImportReviewResponse {
        publication_state: repo.record.publication_state,
        default_visibility: repo.record.default_visibility,
        line_diff: pending_import_line_diff_best_effort(state.object_store.as_ref(), &repo),
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
    let user = optional_scope_user(&state, &headers).await?;
    let principal = principal_for_scope_user(&repo, user.as_ref());
    ensure_repo_read(&state, &repo, &principal)?;
    if repo.has_pending_import_review() {
        ensure_owner(&state, &repo, &principal)?;
    } else if !repo.access_for_principal(&principal).can_apply_changes
        && !repo
            .access_for_principal(&principal)
            .can_change_file_visibility
    {
        return Err(ApiError::forbidden("review permission required"));
    }

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
    let user = optional_scope_user(&state, &headers).await?;
    let repo = find_repo(&state, &owner, &repo_name)?;
    let principal = principal_for_scope_user(&repo, user.as_ref());
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
    state.publish_repo_change(&updated.id, updated.change_version, "repo-published");
    let updated_repo = find_repo(&state, &owner, &repo_name)?;
    let updated_principal = principal_for_scope_user(&updated_repo, user.as_ref());

    Ok(Json(SessionRepo {
        id: updated.id,
        publication_state: updated.publication_state,
        access: repository_access_response(updated_repo.access_for_principal(&updated_principal)),
    }))
}

pub(crate) async fn get_staged_update(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((owner, repo_name)): Path<(String, String)>,
) -> Result<Json<Option<StagedUpdateResponse>>, ApiError> {
    let repo = find_repo(&state, &owner, &repo_name)?;
    let user = optional_scope_user(&state, &headers).await?;
    let principal = principal_for_scope_user(&repo, user.as_ref());
    ensure_repo_read(&state, &repo, &principal)?;
    if !repo.access_for_principal(&principal).can_apply_changes
        && !repo
            .access_for_principal(&principal)
            .can_change_file_visibility
    {
        return Err(ApiError::forbidden("review permission required"));
    }

    let staged = repo.staged_update.as_ref().map(|update| {
        staged_update_response_with_best_effort_diff(state.object_store.as_ref(), update)
    });

    Ok(Json(staged))
}

pub(crate) async fn update_staged_file_visibility(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((owner, repo_name)): Path<(String, String)>,
    Json(input): Json<UpdateStagedFileVisibilityRequest>,
) -> Result<Json<StagedUpdateResponse>, ApiError> {
    let repo = find_repo(&state, &owner, &repo_name)?;
    let user = optional_scope_user(&state, &headers).await?;
    let principal = principal_for_scope_user(&repo, user.as_ref());
    ensure_repo_read(&state, &repo, &principal)?;
    if !repo
        .access_for_principal(&principal)
        .can_change_file_visibility
    {
        return Err(ApiError::forbidden("file visibility permission required"));
    }
    if input.paths.is_empty() {
        return Err(ApiError::bad_request("at least one file path is required"));
    }
    let paths = input
        .paths
        .iter()
        .map(|path| pending_scope_path(path))
        .collect::<Result<Vec<_>, _>>()?;
    let line_diff = repo
        .staged_update
        .as_ref()
        .map(|update| staged_update_line_diff_best_effort(state.object_store.as_ref(), update))
        .unwrap_or_default();

    let updated = state.metadata.update_staged_file_visibility(
        &owner,
        &repo_name,
        &principal.id,
        paths,
        input.visibility,
    )?;
    let repo = find_repo(&state, &owner, &repo_name)?;
    state.publish_repo_change(
        &repo.record.id,
        repo.record.change_version,
        "staged-visibility-changed",
    );

    Ok(Json(staged_update_response(&updated, line_diff)))
}

pub(crate) async fn apply_staged_update(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((owner, repo_name)): Path<(String, String)>,
) -> Result<Json<StagedUpdateResponse>, ApiError> {
    let repo = find_repo(&state, &owner, &repo_name)?;
    let user = optional_scope_user(&state, &headers).await?;
    let principal = principal_for_scope_user(&repo, user.as_ref());
    ensure_repo_read(&state, &repo, &principal)?;
    if !repo.access_for_principal(&principal).can_apply_changes {
        return Err(ApiError::forbidden("apply changes permission required"));
    }
    let line_diff = if let Some(update) = repo.staged_update.as_ref() {
        verify_staged_update_new_blobs(state.object_store.as_ref(), update)?;
        staged_update_line_diff_best_effort(state.object_store.as_ref(), update)
    } else {
        ReviewLineDiffResponse::default()
    };
    let applied = state
        .metadata
        .apply_staged_update(&owner, &repo_name, &principal.id)?;
    let repo = find_repo(&state, &owner, &repo_name)?;
    state.publish_repo_change(
        &repo.record.id,
        repo.record.change_version,
        "staged-update-applied",
    );
    best_effort_drain_pending_source_blob_deletions(&state);

    Ok(Json(staged_update_response(&applied, line_diff)))
}

pub(crate) async fn reject_staged_update(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((owner, repo_name)): Path<(String, String)>,
) -> Result<Json<StagedUpdateResponse>, ApiError> {
    let repo = find_repo(&state, &owner, &repo_name)?;
    let user = optional_scope_user(&state, &headers).await?;
    let principal = principal_for_scope_user(&repo, user.as_ref());
    ensure_repo_read(&state, &repo, &principal)?;
    if !repo.access_for_principal(&principal).can_apply_changes {
        return Err(ApiError::forbidden("apply changes permission required"));
    }
    let line_diff = repo
        .staged_update
        .as_ref()
        .map(|update| staged_update_line_diff_best_effort(state.object_store.as_ref(), update))
        .unwrap_or_default();
    let rejected = state
        .metadata
        .reject_staged_update(&owner, &repo_name, &principal.id)?;
    let repo = find_repo(&state, &owner, &repo_name)?;
    state.publish_repo_change(
        &repo.record.id,
        repo.record.change_version,
        "staged-update-rejected",
    );
    let response = staged_update_response(&rejected, line_diff);
    best_effort_drain_pending_source_blob_deletions(&state);

    Ok(Json(response))
}

fn review_file_diff_response(
    store: &dyn ObjectStore,
    repo: &StoredRepository,
    path: &ScopePath,
) -> Result<ReviewFileDiffResponse, ApiError> {
    if repo.has_pending_import_review() {
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

    review_file_diff_response_for_blobs(
        store,
        file_path.as_str().to_string(),
        StagedFileChangeKind::Added,
        None,
        Some(&file.blob),
    )
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

    review_file_diff_response_for_blobs(
        store,
        change.path.as_str().to_string(),
        change.kind,
        change.old_content.as_ref(),
        change.new_content.as_ref(),
    )
}

fn pending_import_line_diff(
    store: &dyn ObjectStore,
    repo: &StoredRepository,
) -> Result<ReviewLineDiffResponse, ApiError> {
    let mut line_diff = ReviewLineDiffResponse::default();
    if let Some(pending) = &repo.pending_import {
        if summary_line_diff_exceeds_budget(pending.files.iter().map(|file| &file.blob)) {
            return Ok(line_diff);
        }
        for file in &pending.files {
            let file_diff = review_line_diff_for_blobs(store, None, Some(&file.blob))?;
            add_line_diff(&mut line_diff, file_diff);
        }
    }
    Ok(line_diff)
}

fn pending_import_line_diff_best_effort(
    store: &dyn ObjectStore,
    repo: &StoredRepository,
) -> ReviewLineDiffResponse {
    match pending_import_line_diff(store, repo) {
        Ok(line_diff) => line_diff,
        Err(error) => {
            tracing::debug!(
                status = ?error.status,
                message = %error.message,
                "skipping pending import summary line diff"
            );
            ReviewLineDiffResponse::default()
        }
    }
}

fn staged_update_response_with_best_effort_diff(
    store: &dyn ObjectStore,
    update: &StagedRepoUpdate,
) -> StagedUpdateResponse {
    staged_update_response(update, staged_update_line_diff_best_effort(store, update))
}

fn staged_update_line_diff(
    store: &dyn ObjectStore,
    update: &StagedRepoUpdate,
) -> Result<ReviewLineDiffResponse, ApiError> {
    let mut line_diff = ReviewLineDiffResponse::default();
    if summary_line_diff_exceeds_budget(
        update
            .changes
            .iter()
            .flat_map(|change| [change.old_content.as_ref(), change.new_content.as_ref()])
            .flatten(),
    ) {
        return Ok(line_diff);
    }

    for change in &update.changes {
        let change_diff = review_line_diff_for_blobs(
            store,
            change.old_content.as_ref(),
            change.new_content.as_ref(),
        )?;
        add_line_diff(&mut line_diff, change_diff);
    }

    Ok(line_diff)
}

fn verify_staged_update_new_blobs(
    store: &dyn ObjectStore,
    update: &StagedRepoUpdate,
) -> Result<(), ApiError> {
    for change in &update.changes {
        if let Some(blob) = change.new_content.as_ref() {
            source_blob_bytes(store, blob)?;
        }
    }
    Ok(())
}

fn staged_update_line_diff_best_effort(
    store: &dyn ObjectStore,
    update: &StagedRepoUpdate,
) -> ReviewLineDiffResponse {
    match staged_update_line_diff(store, update) {
        Ok(line_diff) => line_diff,
        Err(error) => {
            tracing::debug!(
                status = ?error.status,
                message = %error.message,
                "skipping staged update summary line diff"
            );
            ReviewLineDiffResponse::default()
        }
    }
}

fn summary_line_diff_exceeds_budget<'a>(blobs: impl IntoIterator<Item = &'a SourceBlob>) -> bool {
    const SUMMARY_LINE_DIFF_BLOB_BUDGET: usize = 100;
    const SUMMARY_LINE_DIFF_BYTE_BUDGET: u64 = 1024 * 1024;
    let mut total = 0_u64;
    for (index, blob) in blobs.into_iter().enumerate() {
        if index >= SUMMARY_LINE_DIFF_BLOB_BUDGET {
            return true;
        }
        total = total.saturating_add(blob.size_bytes);
        if total > SUMMARY_LINE_DIFF_BYTE_BUDGET {
            return true;
        }
    }
    false
}
