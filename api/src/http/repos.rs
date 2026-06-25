use crate::domain::git_projection::{VirtualGitProjection, build_virtual_git_projection};
use crate::domain::policy::{Principal, ScopePath, Visibility};
use crate::domain::projection::project_graph;
use crate::domain::store::{RepoRole, RepoSettings};
use crate::{
    auth::{
        clerk::{
            ensure_user_for_identity, http_identity, principal_for_repo, principal_for_user_id,
            require_identity,
        },
        tokens::{generate_first_push_token, generate_git_clone_token, generate_git_push_token},
    },
    error::ApiError,
    http::responses::*,
    http::{
        origins::{public_api_origin, public_app_origin},
        projection_preview::{ensure_projection_preview_access, projection_preview_repo},
    },
    persistence::unix_now,
    state::AppState,
    state::{ensure_repo_read, find_repo, role_for_principal},
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
    let identity = require_identity(&state, &headers).await?;
    let user = ensure_user_for_identity(&state, &identity)?;
    let user_id = user.id.clone();
    let mut repositories = state
        .metadata
        .repositories_for_user(&user_id)?
        .into_iter()
        .filter(|repo| {
            repo.record.publication_state
                != crate::domain::store::RepoPublicationState::PendingFirstPush
        })
        .filter_map(|repo| repo_summary_for_user(&repo, &user_id))
        .collect::<Vec<_>>();
    repositories.sort_by(|left, right| left.id.cmp(&right.id));

    Ok(Json(repositories))
}

pub(crate) async fn create_repo(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(input): Json<CreateRepoRequest>,
) -> Result<Json<CreateRepoResponse>, ApiError> {
    let identity = require_identity(&state, &headers).await?;
    let user = ensure_user_for_identity(&state, &identity)?;
    let default_visibility = input.visibility.unwrap_or(Visibility::Private);
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
    let api_origin = public_api_origin()?;
    let app_origin = public_app_origin("create repository init metadata")?;
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
    let repo = find_repo(&state, &owner, &repo_name)?;
    let identity = http_identity(&state, &headers).await?;
    let principal = principal_for_repo(&state, &repo, identity.as_ref())?;
    ensure_repo_read(&state, &repo, &principal)?;
    let role = role_for_principal(&state, &repo, &principal)?;
    let staged_update_pending = role == Some(RepoRole::Owner) && repo.staged_update.is_some();
    let summary = RepoSummaryResponse {
        id: repo.record.id.clone(),
        owner_handle: repo.record.owner_handle.clone(),
        name: repo.record.name.clone(),
        lifecycle_state: repo.record.publication_state,
        default_visibility: repo.record.default_visibility,
        role,
        staged_update_pending,
    };

    Ok(Json(summary))
}

pub(crate) async fn delete_repo(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((owner, repo_name)): Path<(String, String)>,
) -> Result<Json<DeleteRepoResponse>, ApiError> {
    let identity = require_identity(&state, &headers).await?;
    let user = ensure_user_for_identity(&state, &identity)?;
    let repo_id = state.metadata.delete_repo(&owner, &repo_name, &user.id)?;

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
    let identity = require_identity(&state, &headers).await?;
    let user = ensure_user_for_identity(&state, &identity)?;
    let repo = find_repo(&state, &owner, &repo_name)?;
    let principal = principal_for_user_id(&repo, &user.id);
    ensure_repo_read(&state, &repo, &principal)?;
    if role_for_principal(&state, &repo, &principal)?.is_none() {
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

pub(crate) async fn get_projection(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((owner, repo_name)): Path<(String, String)>,
) -> Result<Json<ProjectionResponse>, ApiError> {
    let repo = find_repo(&state, &owner, &repo_name)?;
    let identity = http_identity(&state, &headers).await?;
    let principal = principal_for_repo(&state, &repo, identity.as_ref())?;
    ensure_repo_read(&state, &repo, &principal)?;
    let projection = project_graph(&repo.policy, &repo.graph, &principal);
    Ok(Json(projection_response(
        state.object_store.as_ref(),
        projection,
    )?))
}

pub(crate) async fn get_git_projection(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((owner, repo_name)): Path<(String, String)>,
) -> Result<Json<VirtualGitProjection>, ApiError> {
    let repo = find_repo(&state, &owner, &repo_name)?;
    let identity = http_identity(&state, &headers).await?;
    let principal = principal_for_repo(&state, &repo, identity.as_ref())?;
    ensure_repo_read(&state, &repo, &principal)?;
    let projection = project_graph(&repo.policy, &repo.graph, &principal);
    Ok(Json(build_virtual_git_projection(
        state.object_store.as_ref(),
        &projection,
    )?))
}

pub(crate) async fn get_projection_preview(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((owner, repo_name)): Path<(String, String)>,
    Query(input): Query<ProjectionPreviewRequest>,
) -> Result<Json<ProjectionPreviewResponse>, ApiError> {
    let repo = find_repo(&state, &owner, &repo_name)?;
    let source = input.source.unwrap_or(ProjectionPreviewSource::Live);
    let identity = http_identity(&state, &headers).await?;
    let requester = principal_for_repo(&state, &repo, identity.as_ref())?;
    ensure_projection_preview_access(&state, &repo, &requester, input.audience, source)?;
    let include_private_counts =
        role_for_principal(&state, &repo, &requester)? == Some(RepoRole::Owner);
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
    let repo = find_repo(&state, &owner, &repo_name)?;
    let identity = http_identity(&state, &headers).await?;
    let principal = principal_for_repo(&state, &repo, identity.as_ref())?;
    ensure_repo_read(&state, &repo, &principal)?;

    Ok(Json(projected_files(&repo, &principal)?))
}

pub(crate) async fn update_file_visibility(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((owner, repo_name)): Path<(String, String)>,
    Json(input): Json<UpdateFileVisibilityRequest>,
) -> Result<Json<Vec<RepoFileResponse>>, ApiError> {
    let identity = http_identity(&state, &headers).await?;
    let user = identity
        .as_ref()
        .map(|identity| ensure_user_for_identity(&state, identity))
        .transpose()?;
    let update_paths = parse_visibility_paths(&input.paths)?;
    let visibility = input.visibility;
    let repo = find_repo(&state, &owner, &repo_name)?;
    let principal = user
        .as_ref()
        .map(|user| principal_for_user_id(&repo, &user.id))
        .unwrap_or_else(Principal::public);
    ensure_repo_read(&state, &repo, &principal)?;
    if role_for_principal(&state, &repo, &principal)? != Some(RepoRole::Owner) {
        return Err(ApiError::forbidden("owner role required"));
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

    let principal = Principal {
        id: updated.record.owner_user_id.clone(),
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
    let repo = find_repo(&state, &owner, &repo_name)?;
    let identity = http_identity(&state, &headers).await?;
    let principal = principal_for_repo(&state, &repo, identity.as_ref())?;
    ensure_repo_read(&state, &repo, &principal)?;

    if role_for_principal(&state, &repo, &principal)? != Some(RepoRole::Owner) {
        return Err(ApiError::forbidden("owner role required"));
    }

    Ok(Json(RepoSettingsResponse {
        default_new_file_visibility: repo.record.default_visibility,
        review_pushes_before_applying: repo.settings.review_pushes_before_applying,
    }))
}

pub(crate) async fn update_settings(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((owner, repo_name)): Path<(String, String)>,
    Json(input): Json<UpdateRepoSettingsRequest>,
) -> Result<Json<RepoSettingsResponse>, ApiError> {
    let identity = http_identity(&state, &headers).await?;
    let user = identity
        .as_ref()
        .map(|identity| ensure_user_for_identity(&state, identity))
        .transpose()?;
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

    Ok(Json(RepoSettingsResponse {
        default_new_file_visibility: updated.record.default_visibility,
        review_pushes_before_applying: updated.settings.review_pushes_before_applying,
    }))
}
