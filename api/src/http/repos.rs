use crate::domain::policy::{ScopePath, Visibility};
use crate::domain::repo_actions::reviewed_update_api_error;
use crate::domain::repo_config::is_repo_config_fingerprint;
use crate::domain::requests::{Request, RequestState, request_visible_to_access};
use crate::domain::reviewed_updates::{ReviewedConfigUpdateInput, apply_reviewed_config_to_repo};
use crate::domain::store::{RepositoryAccess, RepositoryActor};
use crate::{
    auth::{
        scope::{optional_scope_user, principal_for_scope_user, require_scope_user},
        tokens::{generate_first_push_token, generate_git_push_token},
    },
    db::{RepoSummaryRead, RepositoryMutation},
    error::ApiError,
    http::responses::*,
    http::{
        origins::public_api_origin,
        projection_preview::{ensure_projection_preview_access, projection_preview_repo},
    },
    persistence::unix_now,
    state::AppState,
    state::{find_repo, repo_config_fingerprint},
};
use axum::{
    Json,
    extract::{Path, Query, State},
    http::HeaderMap,
};

const MAX_PUSH_INTENT_CONFIG_BYTES: usize = 4096;

pub(crate) async fn list_repos(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<Vec<RepoSummaryResponse>>, ApiError> {
    let user = require_scope_user(&state, &headers).await?;
    let user_id = user.id.clone();
    let mut repositories = Vec::new();
    for summary in state.metadata.repo_summaries_for_user(&user_id).await? {
        repositories.push(repo_summary_response(&state, summary).await?);
    }
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
    let cleanup_state = state.clone();
    let (secret, token) = generate_first_push_token(&user.id)?;
    let (push_secret, push_token) = generate_git_push_token(&user.id)?;
    let now = unix_now()?;

    let repo = state
        .metadata
        .create_repo_with_init_tokens(
            &user.id,
            &input.name,
            default_visibility,
            token,
            push_token,
            move |owner_handle, repo_name| {
                crate::git::storage::delete_repo_storage(&cleanup_state, owner_handle, repo_name)
                    .map_err(Into::into)
            },
        )
        .await?;

    let user_id = user.id.clone();
    let summary = repo_summary_for_user(&repo, &user_id, 0)
        .ok_or_else(|| ApiError::internal_message("created repository is missing owner role"))?;
    let init = repo_init_response(
        &repo,
        &user_id,
        &api_origin,
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
        )
        .await?
        .ok_or_else(|| ApiError::not_found(format!("repo {owner}/{repo_name} not found")))?;

    Ok(Json(repo_summary_response(&state, summary).await?))
}

pub(crate) async fn delete_repo(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((owner, repo_name)): Path<(String, String)>,
) -> Result<Json<DeleteRepoResponse>, ApiError> {
    let user = require_scope_user(&state, &headers).await?;
    let repo = find_repo(&state, &owner, &repo_name).await?;
    let delete_version = repo.record.change_version.saturating_add(1);
    let repo_id = state
        .metadata
        .delete_repo(&owner, &repo_name, &user.id)
        .await?;
    state
        .publish_repo_change(&repo_id, delete_version, "repo-deleted")
        .await;

    crate::state::best_effort_drain_pending_repo_storage_deletions(&state).await;
    Ok(Json(DeleteRepoResponse {
        id: repo_id,
        deleted: true,
    }))
}

pub(crate) async fn get_repo_config(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((owner, repo_name)): Path<(String, String)>,
) -> Result<Json<RepoConfigResponse>, ApiError> {
    let user = require_scope_user(&state, &headers).await?;
    let repo = state
        .metadata
        .git_push_context(&owner, &repo_name, &user.id)
        .await?
        .ok_or_else(|| ApiError::not_found(format!("repo {owner}/{repo_name} not found")))?;
    if repo.access.actor == RepositoryActor::Public {
        let full_repo = find_repo(&state, &owner, &repo_name).await?;
        let principal = principal_for_scope_user(&full_repo, Some(&user));
        crate::state::ensure_repo_read(&state, &full_repo, &principal)?;
        return Err(ApiError::forbidden("repo membership required"));
    }

    Ok(Json(RepoConfigResponse {
        config_hash: repo_config_fingerprint(&repo.repo_config)?,
        lifecycle_state: repo.publication_state,
        access: repository_access_response(repo.access),
        head_oid: repo.git_head.as_ref().map(|head| head.head_oid.clone()),
        config: repo.repo_config,
    }))
}

pub(crate) async fn create_push_intent(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((owner, repo_name)): Path<(String, String)>,
    Json(input): Json<CreatePushIntentRequest>,
) -> Result<Json<CreatePushIntentResponse>, ApiError> {
    let user = require_scope_user(&state, &headers).await?;
    let repo = state
        .metadata
        .git_push_context(&owner, &repo_name, &user.id)
        .await?
        .ok_or_else(|| ApiError::not_found(format!("repo {owner}/{repo_name} not found")))?;
    let access = repo.access;

    if repo.publication_state == crate::domain::store::RepoPublicationState::Unpublished {
        if access.actor != RepositoryActor::Owner {
            return Err(ApiError::not_found(format!(
                "repo {owner}/{repo_name} not found"
            )));
        }
    } else if !access.can_push {
        return Err(ApiError::not_found(format!(
            "repo {owner}/{repo_name} not found"
        )));
    }

    let head_oid = git_oid_request("head_oid", &input.head_oid)?;
    validate_push_intent_config_transport(&input.config)?;
    let base_config_hash = repo_config_fingerprint(&repo.repo_config)?;
    if !is_repo_config_fingerprint(&input.base_config_hash) {
        return Err(ApiError::bad_request(
            "base_config_hash must be a SHA-256 hex digest",
        ));
    }
    if base_config_hash != input.base_config_hash && repo.repo_config != input.config {
        return Err(ApiError::conflict(
            "repo config changed since review; rerun scope review",
        ));
    }
    let base_head_oid = repo.git_head.as_ref().map(|head| head.head_oid.clone());
    let base_git_manifest_key = repo
        .git_head
        .as_ref()
        .map(|head| head.manifest.object_key.clone());
    let config_changed = repo.repo_config != input.config;
    if base_head_oid.as_deref() == Some(head_oid.as_str()) && config_changed {
        let author_id = user.id.clone();
        let config = input.config.clone();
        let expected_config_hash = base_config_hash.clone();
        let expected_manifest_key = base_git_manifest_key.clone();
        let changed = state
            .metadata
            .mutate_repository(&owner, &repo_name, move |repo| {
                let access = repo.access_for_user_id(&author_id);
                if !access.can_push {
                    return Err(ApiError::forbidden("push permission required").into());
                }
                if !access.can_change_file_visibility && repo.repo_config != config {
                    return Err(ApiError::forbidden("file visibility permission required").into());
                }
                if repo
                    .git_head
                    .as_ref()
                    .map(|head| head.manifest.object_key.clone())
                    != expected_manifest_key
                {
                    return Err(ApiError::conflict(
                        "repo content changed since review; rerun scope push",
                    )
                    .into());
                }
                if repo_config_fingerprint(&repo.repo_config)? != expected_config_hash {
                    return Err(ApiError::conflict(
                        "repo config changed since review; rerun scope push",
                    )
                    .into());
                }
                let changed = apply_reviewed_config_to_repo(
                    repo,
                    ReviewedConfigUpdateInput { author_id, config },
                )
                .map_err(reviewed_update_api_error)?;
                Ok(RepositoryMutation::new(changed))
            })
            .await?;
        if changed {
            let repo = state
                .metadata
                .git_push_context(&owner, &repo_name, &user.id)
                .await?
                .ok_or_else(|| {
                    ApiError::not_found(format!("repo {owner}/{repo_name} not found"))
                })?;
            state
                .publish_repo_change(&repo.repo_id, repo.change_version, "config-applied")
                .await;
        }
    }
    let intent_base_config_hash =
        if config_changed && base_head_oid.as_deref() == Some(head_oid.as_str()) {
            repo_config_fingerprint(&input.config)?
        } else {
            base_config_hash
        };
    let intent = state.create_push_intent(
        &repo.repo_id,
        &user.id,
        &head_oid,
        input.config,
        intent_base_config_hash,
        base_git_manifest_key,
    )?;

    Ok(Json(CreatePushIntentResponse {
        token: intent.token,
        base_head_oid: base_head_oid.map(git_oid_response).transpose()?,
        expires_at_unix: intent.expires_at_unix,
    }))
}

fn validate_push_intent_config_transport(
    config: &crate::domain::repo_config::RepoConfig,
) -> Result<(), ApiError> {
    config.validate().map_err(ApiError::bad_request)?;
    let bytes = serde_json::to_vec(config).map_err(ApiError::internal)?;
    if bytes.len() > MAX_PUSH_INTENT_CONFIG_BYTES {
        return Err(ApiError::bad_request(format!(
            "repo config exceeds {MAX_PUSH_INTENT_CONFIG_BYTES} bytes"
        )));
    }
    Ok(())
}

pub(crate) async fn get_projection_preview(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((owner, repo_name)): Path<(String, String)>,
    Query(input): Query<ProjectionPreviewRequest>,
) -> Result<Json<ProjectionPreviewResponse>, ApiError> {
    let repo = find_repo(&state, &owner, &repo_name).await?;
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
        )
        .await?
        .ok_or_else(|| ApiError::not_found(format!("repo {owner}/{repo_name} not found")))?;

    Ok(Json(projection_file_responses(files)))
}

pub(crate) async fn get_file_content(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((owner, repo_name)): Path<(String, String)>,
    Query(input): Query<RepoFileContentRequest>,
) -> Result<Json<RepoFileContentResponse>, ApiError> {
    let path = ScopePath::parse(format!("/{}", input.path)).map_err(ApiError::bad_request)?;
    if path == ScopePath::root() {
        return Err(ApiError::bad_request("file path is required"));
    }
    let user = optional_scope_user(&state, &headers).await?;
    let projected = state
        .metadata
        .repo_live_file_content(
            &owner,
            &repo_name,
            user.as_ref().map(|user| user.id.as_str()),
            &path,
        )
        .await?
        .ok_or_else(|| ApiError::not_found("file not found"))?;
    let content =
        crate::http::file_diffs::review_content_response_for_blob(&state, &projected.blob)?;

    Ok(Json(RepoFileContentResponse {
        path: projected.file.path.as_str().to_string(),
        oid: projected.file.oid,
        visibility: projected.file.visibility,
        size_bytes: projected.blob.size_bytes,
        content,
    }))
}

async fn repo_summary_response(
    state: &AppState,
    summary: RepoSummaryRead,
) -> Result<RepoSummaryResponse, ApiError> {
    let open_request_count = state
        .metadata
        .requests_by_repo_id(&summary.id)
        .await?
        .into_iter()
        .filter(|request| request_visible_in_summary(request, summary.access))
        .count();
    let request_permissions = repo_request_permissions_response(summary.access);
    Ok(RepoSummaryResponse {
        id: summary.id,
        owner_handle: summary.owner_handle,
        name: summary.name,
        lifecycle_state: summary.lifecycle_state,
        default_visibility: summary.default_visibility,
        change_version: summary.change_version,
        access: repository_access_response(summary.access),
        open_request_count,
        request_permissions,
    })
}

fn request_visible_in_summary(request: &Request, access: RepositoryAccess) -> bool {
    if matches!(
        request.state,
        RequestState::Working | RequestState::Resolved | RequestState::Withdrawn
    ) {
        return false;
    }
    request_visible_to_access(request, access)
}
