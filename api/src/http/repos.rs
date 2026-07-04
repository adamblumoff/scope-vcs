use crate::domain::policy::Visibility;
use crate::domain::store::RepositoryActor;
use crate::{
    auth::{
        scope::{
            optional_scope_user, principal_for_scope_user, principal_for_user_id,
            require_scope_user,
        },
        tokens::{generate_first_push_token, generate_git_clone_token, generate_git_push_token},
    },
    db::RepoSummaryRead,
    error::ApiError,
    http::responses::*,
    http::{
        origins::public_api_origin,
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

pub(crate) async fn create_push_intent(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((owner, repo_name)): Path<(String, String)>,
    Json(input): Json<CreatePushIntentRequest>,
) -> Result<Json<CreatePushIntentResponse>, ApiError> {
    let user = require_scope_user(&state, &headers).await?;
    let repo = find_repo(&state, &owner, &repo_name)?;
    let principal = principal_for_user_id(&repo, &user.id);
    let access = repo.access_for_principal(&principal);

    if repo.is_waiting_for_first_push() {
        if access.actor != RepositoryActor::Owner {
            return Err(ApiError::not_found(format!(
                "repo {owner}/{repo_name} not found"
            )));
        }
    } else if repo.record.publication_state == crate::domain::store::RepoPublicationState::Published
    {
        if !access.can_push {
            return Err(ApiError::not_found(format!(
                "repo {owner}/{repo_name} not found"
            )));
        }
    } else {
        if access.actor != RepositoryActor::Owner {
            return Err(ApiError::not_found(format!(
                "repo {owner}/{repo_name} not found"
            )));
        }
        return Err(ApiError::conflict(
            "repo has a stale pending import; resolve it before creating a push intent",
        ));
    }

    if repo.has_pending_import_review() || repo.staged_update.is_some() {
        return Err(ApiError::conflict(
            "repo has stale pending push state; retry after cleanup",
        ));
    }

    let head_oid = normalize_git_oid(&input.head_oid)?;
    let base_head_oid = git_snapshot_head_oid(&state, repo.git_snapshot.as_ref())?;
    let intent = state.create_push_intent(
        &repo.record.id,
        &user.id,
        &head_oid,
        repo.git_snapshot
            .as_ref()
            .map(|snapshot| snapshot.object_key.clone()),
    )?;

    Ok(Json(CreatePushIntentResponse {
        token: intent.token,
        base_head_oid,
        expires_at_unix: intent.expires_at_unix,
    }))
}

fn git_snapshot_head_oid(
    state: &AppState,
    snapshot: Option<&crate::domain::store::SourceBlob>,
) -> Result<Option<String>, ApiError> {
    let Some(snapshot) = snapshot else {
        return Ok(None);
    };

    let repo_path = crate::git::storage::cached_raw_git_snapshot_repo(state, snapshot)?;
    let branch_ref = format!("refs/heads/{}", crate::config::DEFAULT_GIT_BRANCH);
    let refs = crate::git::import::git_refs(&repo_path)?;
    Ok(refs.into_iter().find_map(|(refname, oid)| {
        if refname == branch_ref {
            Some(oid)
        } else {
            None
        }
    }))
}

fn normalize_git_oid(value: &str) -> Result<String, ApiError> {
    let oid = value.trim();
    // Scope stores raw Git snapshots as SHA-1 repositories today.
    if oid.len() == 40 && oid.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        Ok(oid.to_ascii_lowercase())
    } else {
        Err(ApiError::bad_request(
            "head_oid must be a full SHA-1 Git object id",
        ))
    }
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
