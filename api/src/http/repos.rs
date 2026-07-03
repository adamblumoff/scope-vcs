use crate::domain::policy::{Principal, ScopePath, Visibility};
use crate::domain::store::{RepoSettings, RepositoryActor};
use crate::{
    auth::{
        scope::{
            optional_scope_user, principal_for_scope_user, principal_for_user_id,
            require_scope_user,
        },
        tokens::{generate_first_push_token, generate_git_clone_token, generate_git_push_token},
    },
    db::{RepoSettingsRead, RepoSummaryRead},
    error::ApiError,
    http::responses::*,
    http::{
        origins::{public_api_origin, public_app_origin},
        projection_preview::{ensure_projection_preview_access, projection_preview_repo},
    },
    persistence::unix_now,
    state::AppState,
    state::{ensure_repo_read, find_repo},
};
use axum::{
    Json,
    extract::{Path, Query, State},
    http::HeaderMap,
};
use std::collections::BTreeSet;

pub(crate) async fn list_repos(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<Vec<RepoSummaryResponse>>, ApiError> {
    let user = require_scope_user(&state, &headers).await?;
    let user_id = user.id.clone();
    let mut repositories = state
        .metadata
        .repo_summaries_for_user(&user_id)?
        .into_iter()
        .map(repo_summary_response)
        .collect::<Vec<_>>();
    repositories.sort_by(|left, right| left.id.cmp(&right.id));

    Ok(Json(repositories))
}

pub(crate) async fn create_repo(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(input): Json<CreateRepoRequest>,
) -> Result<Json<CreateRepoResponse>, ApiError> {
    let user = require_scope_user(&state, &headers).await?;
    let default_visibility = input.visibility.unwrap_or(Visibility::Private);
    let api_origin = public_api_origin()?;
    let app_origin = public_app_origin("create repository init metadata")?;
    let cleanup_state = state.clone();
    let (secret, token) = generate_first_push_token(&user.id)?;
    let (push_secret, push_token) = generate_git_push_token(&user.id)?;
    let now = unix_now()?;

    let repo = state.metadata.create_repo_with_init_tokens(
        &user.id,
        &input.name,
        default_visibility,
        token,
        push_token,
        move |owner_handle, repo_name| {
            crate::git::storage::delete_repo_storage(&cleanup_state, owner_handle, repo_name)
        },
    )?;

    let user_id = user.id.clone();
    let summary = repo_summary_for_user(&repo, &user_id)
        .ok_or_else(|| ApiError::internal_message("created repository is missing owner role"))?;
    let init = repo_init_response(
        &repo,
        &user_id,
        &api_origin,
        &app_origin,
        now,
        Some(secret),
        Some(push_secret),
    )?;

    let created = CreateRepoResponse {
        repo: summary,
        init,
    };

    Ok(Json(created))
}

pub(crate) async fn get_repo(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((owner, repo_name)): Path<(String, String)>,
) -> Result<Json<RepoSummaryResponse>, ApiError> {
    let user = optional_scope_user(&state, &headers).await?;
    let summary = state
        .metadata
        .repo_summary(
            &owner,
            &repo_name,
            user.as_ref().map(|user| user.id.as_str()),
        )?
        .ok_or_else(|| ApiError::not_found(format!("repo {owner}/{repo_name} not found")))?;

    Ok(Json(repo_summary_response(summary)))
}

pub(crate) async fn delete_repo(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((owner, repo_name)): Path<(String, String)>,
) -> Result<Json<DeleteRepoResponse>, ApiError> {
    let user = require_scope_user(&state, &headers).await?;
    let repo = find_repo(&state, &owner, &repo_name)?;
    let delete_version = repo.record.change_version.saturating_add(1);
    let repo_id = state.metadata.delete_repo(&owner, &repo_name, &user.id)?;
    state.publish_repo_change(&repo_id, delete_version, "repo-deleted");

    crate::state::best_effort_drain_pending_repo_storage_deletions(&state);
    crate::state::best_effort_drain_pending_source_blob_deletions(&state);

    Ok(Json(DeleteRepoResponse {
        id: repo_id,
        deleted: true,
    }))
}

pub(crate) async fn create_clone_credential(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((owner, repo_name)): Path<(String, String)>,
) -> Result<Json<RepoCloneCredentialResponse>, ApiError> {
    let user = require_scope_user(&state, &headers).await?;
    let repo = find_repo(&state, &owner, &repo_name)?;
    let principal = principal_for_user_id(&repo, &user.id);
    ensure_repo_read(&state, &repo, &principal)?;
    if repo.access_for_principal(&principal).actor == RepositoryActor::Public {
        return Err(ApiError::forbidden("repo membership required"));
    }

    let (secret, token) = generate_git_clone_token(&user.id)?;
    let token = state
        .metadata
        .create_git_clone_token(&owner, &repo_name, &user.id, token)?;

    Ok(Json(repo_clone_credential_response(
        &repo,
        &token,
        Some(secret),
    )))
}

pub(crate) async fn get_projection_preview(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((owner, repo_name)): Path<(String, String)>,
    Query(input): Query<ProjectionPreviewRequest>,
) -> Result<Json<ProjectionPreviewResponse>, ApiError> {
    let repo = find_repo(&state, &owner, &repo_name)?;
    let source = input.source.unwrap_or(ProjectionPreviewSource::Live);
    let user = optional_scope_user(&state, &headers).await?;
    let requester = principal_for_scope_user(&repo, user.as_ref());
    ensure_projection_preview_access(&state, &repo, &requester, input.audience, source)?;
    let include_private_counts =
        repo.access_for_principal(&requester).actor != RepositoryActor::Public;
    let preview_repo = projection_preview_repo(&repo, source)?;

    Ok(Json(projection_preview_response(
        &preview_repo,
        input.audience,
        source,
        include_private_counts,
    )?))
}

pub(crate) async fn get_files(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((owner, repo_name)): Path<(String, String)>,
) -> Result<Json<Vec<RepoFileResponse>>, ApiError> {
    let user = optional_scope_user(&state, &headers).await?;
    let files = state
        .metadata
        .repo_live_files(
            &owner,
            &repo_name,
            user.as_ref().map(|user| user.id.as_str()),
        )?
        .ok_or_else(|| ApiError::not_found(format!("repo {owner}/{repo_name} not found")))?;

    Ok(Json(projection_file_responses(files)))
}

pub(crate) async fn update_file_visibility(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((owner, repo_name)): Path<(String, String)>,
    Json(input): Json<UpdateFileVisibilityRequest>,
) -> Result<Json<Vec<RepoFileResponse>>, ApiError> {
    let user = optional_scope_user(&state, &headers).await?;
    let update_paths = parse_visibility_paths(&input.paths)?;
    let visibility = input.visibility;
    let repo = find_repo(&state, &owner, &repo_name)?;
    let principal = user
        .as_ref()
        .map(|user| principal_for_user_id(&repo, &user.id))
        .unwrap_or_else(Principal::public);
    ensure_repo_read(&state, &repo, &principal)?;
    if !repo
        .access_for_principal(&principal)
        .can_change_file_visibility
    {
        return Err(ApiError::forbidden("file visibility permission required"));
    }

    let owner_files = files_for_visibility_update(&repo, &principal)?;
    for path in &update_paths {
        let selected_file = owner_files
            .iter()
            .find(|file| file.path == path.as_str())
            .ok_or_else(|| ApiError::not_found(format!("file {} not found", path.as_str())))?;
        if visibility == Visibility::Public && !selected_file.tracked {
            return Err(ApiError::bad_request(format!(
                "file {} must be tracked by Git before it can be made public",
                path.as_str()
            )));
        }
    }

    let user_id = user
        .as_ref()
        .map(|user| user.id.clone())
        .ok_or_else(|| ApiError::forbidden("owner role required"))?;
    let updated = state.metadata.update_repo_file_visibility(
        &owner,
        &repo_name,
        &user_id,
        update_paths,
        visibility,
    )?;
    state.publish_repo_change(
        &updated.record.id,
        updated.record.change_version,
        "visibility-changed",
    );

    let principal = Principal {
        id: user_id,
        kind: crate::domain::policy::PrincipalKind::User,
    };
    let updated_files = files_for_visibility_update(&updated, &principal)?;

    Ok(Json(updated_files))
}

fn parse_visibility_paths(paths: &[String]) -> Result<Vec<ScopePath>, ApiError> {
    if paths.is_empty() {
        return Err(ApiError::bad_request("at least one file path is required"));
    }

    let mut parsed = BTreeSet::new();
    for path in paths {
        parsed.insert(ScopePath::parse(path).map_err(ApiError::bad_request)?);
    }

    Ok(parsed.into_iter().collect())
}

pub(crate) async fn get_settings(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((owner, repo_name)): Path<(String, String)>,
) -> Result<Json<RepoSettingsResponse>, ApiError> {
    let user = optional_scope_user(&state, &headers).await?;
    let settings = state
        .metadata
        .repo_settings(
            &owner,
            &repo_name,
            user.as_ref().map(|user| user.id.as_str()),
        )?
        .ok_or_else(|| ApiError::not_found(format!("repo {owner}/{repo_name} not found")))?;

    Ok(Json(repo_settings_response(settings)))
}

pub(crate) async fn update_settings(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((owner, repo_name)): Path<(String, String)>,
    Json(input): Json<UpdateRepoSettingsRequest>,
) -> Result<Json<RepoSettingsResponse>, ApiError> {
    let user = optional_scope_user(&state, &headers).await?;
    let repo = find_repo(&state, &owner, &repo_name)?;
    let principal = user
        .as_ref()
        .map(|user| principal_for_user_id(&repo, &user.id))
        .unwrap_or_else(Principal::public);
    ensure_repo_read(&state, &repo, &principal)?;

    let user_id = user
        .as_ref()
        .map(|user| user.id.clone())
        .ok_or_else(|| ApiError::forbidden("owner role required"))?;
    let updated = state.metadata.update_repo_settings(
        &owner,
        &repo_name,
        &user_id,
        RepoSettings {
            include_ignored_files: repo.settings.include_ignored_files,
            review_pushes_before_applying: input.review_pushes_before_applying,
        },
        input.default_new_file_visibility,
    )?;
    state.publish_repo_change(
        &updated.record.id,
        updated.record.change_version,
        "settings-changed",
    );

    Ok(Json(RepoSettingsResponse {
        default_new_file_visibility: updated.record.default_visibility,
        review_pushes_before_applying: updated.settings.review_pushes_before_applying,
    }))
}

fn repo_summary_response(summary: RepoSummaryRead) -> RepoSummaryResponse {
    RepoSummaryResponse {
        id: summary.id,
        owner_handle: summary.owner_handle,
        name: summary.name,
        lifecycle_state: summary.lifecycle_state,
        default_visibility: summary.default_visibility,
        change_version: summary.change_version,
        access: repository_access_response(summary.access),
        pending_import_pending: summary.pending_import_pending,
        staged_update_pending: summary.staged_update_pending,
        push_blocked_by_staged_update: summary.push_blocked_by_staged_update,
    }
}

fn repo_settings_response(settings: RepoSettingsRead) -> RepoSettingsResponse {
    RepoSettingsResponse {
        default_new_file_visibility: settings.default_new_file_visibility,
        review_pushes_before_applying: settings.review_pushes_before_applying,
    }
}
