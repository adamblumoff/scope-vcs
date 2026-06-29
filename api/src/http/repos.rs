use crate::domain::policy::{Principal, ScopePath, Visibility};
use crate::domain::projection::project_graph;
use crate::domain::store::{RepoSettings, RepositoryActor};
use crate::{
    auth::{
        scope::{
            optional_scope_user, principal_for_scope_user, principal_for_user_id,
            require_scope_user,
        },
        tokens::{
            generate_first_push_token, generate_git_clone_token, generate_git_push_token,
            generate_repository_invite_token, repository_invite_token_hash,
        },
    },
    error::ApiError,
    http::responses::*,
    http::{
        origins::{public_api_origin, public_app_origin},
        projection_preview::{ensure_projection_preview_access, projection_preview_repo},
    },
    persistence::unix_now,
    state::AppState,
    state::{access_for_principal, ensure_repo_read, find_repo},
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
        .repositories_for_user(&user_id)?
        .into_iter()
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
    let repo = find_repo(&state, &owner, &repo_name)?;
    let user = optional_scope_user(&state, &headers).await?;
    let principal = principal_for_scope_user(&repo, user.as_ref());
    ensure_repo_read(&state, &repo, &principal)?;
    let access = access_for_principal(&state, &repo, &principal)?;
    let summary = RepoSummaryResponse {
        id: repo.record.id.clone(),
        owner_handle: repo.record.owner_handle.clone(),
        name: repo.record.name.clone(),
        lifecycle_state: repo.record.publication_state,
        default_visibility: repo.record.default_visibility,
        change_version: repo_change_version_for_access(&repo, access),
        access: repository_access_response(access),
        pending_import_pending: repo.has_pending_import_review(),
        staged_update_pending: access.can_apply_changes && repo.staged_update.is_some(),
        push_blocked_by_staged_update: access.can_push && repo.staged_update.is_some(),
    };

    Ok(Json(summary))
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

pub(crate) async fn get_projection(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((owner, repo_name)): Path<(String, String)>,
) -> Result<Json<ProjectionResponse>, ApiError> {
    let repo = find_repo(&state, &owner, &repo_name)?;
    let user = optional_scope_user(&state, &headers).await?;
    let principal = principal_for_scope_user(&repo, user.as_ref());
    ensure_repo_read(&state, &repo, &principal)?;
    let projection = project_graph(
        &repo.policy,
        &repo.graph,
        &repo.visibility_events,
        &principal,
        repo.access_for_principal(&principal).can_read_private_files,
    );
    Ok(Json(projection_response(
        state.object_store.as_ref(),
        projection,
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
    let user = optional_scope_user(&state, &headers).await?;
    let requester = principal_for_scope_user(&repo, user.as_ref());
    ensure_projection_preview_access(&state, &repo, &requester, input.audience, source)?;
    let include_private_counts =
        repo.access_for_principal(&requester).actor == RepositoryActor::Owner;
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
    let user = optional_scope_user(&state, &headers).await?;
    let principal = principal_for_scope_user(&repo, user.as_ref());
    ensure_repo_read(&state, &repo, &principal)?;

    Ok(Json(projected_files(&repo, &principal)?))
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
    let repo = find_repo(&state, &owner, &repo_name)?;
    let user = optional_scope_user(&state, &headers).await?;
    let principal = principal_for_scope_user(&repo, user.as_ref());
    ensure_repo_read(&state, &repo, &principal)?;

    if repo.access_for_principal(&principal).actor != RepositoryActor::Owner {
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

pub(crate) async fn list_repository_collaboration(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((owner, repo_name)): Path<(String, String)>,
) -> Result<Json<RepositoryCollaborationResponse>, ApiError> {
    let user = require_scope_user(&state, &headers).await?;
    let owner_for_read = owner.clone();
    let repo_for_read = repo_name.clone();
    let response = state.metadata.read(move |catalog| {
        let repo = catalog
            .repository(&owner_for_read, &repo_for_read)
            .ok_or_else(|| {
                ApiError::not_found(format!("repo {owner_for_read}/{repo_for_read} not found"))
            })?;
        if !repo.is_owner_user(&user.id) {
            return Err(ApiError::forbidden("owner role required"));
        }
        Ok(repository_collaboration_response(repo, &catalog.users))
    })?;

    Ok(Json(response))
}

pub(crate) async fn create_repository_invite(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((owner, repo_name)): Path<(String, String)>,
    Json(input): Json<CreateRepositoryInviteRequest>,
) -> Result<Json<CreateRepositoryInviteResponse>, ApiError> {
    let user = require_scope_user(&state, &headers).await?;
    let (secret, token_hash) = generate_repository_invite_token()?;
    let now = unix_now()?;
    let invite_id = format!("repo_invite_{}", token_hash.replace([':', '/'], "_"));
    let invite = state.metadata.create_repository_invite(
        &owner,
        &repo_name,
        user,
        input.email,
        input.permissions,
        invite_id,
        token_hash,
        now,
    )?;
    let repo = find_repo(&state, &owner, &repo_name)?;
    state.publish_repo_change(
        &repo.record.id,
        repo.record.change_version,
        "invite-updated",
    );
    let app_origin = public_app_origin("building repository invite URL")?;

    Ok(Json(CreateRepositoryInviteResponse {
        invite: repository_invite_response(&invite),
        invite_url: format!("{}/invites/{}", app_origin.trim_end_matches('/'), secret),
    }))
}

pub(crate) async fn update_repository_member(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((owner, repo_name, member_user_id)): Path<(String, String, String)>,
    Json(input): Json<UpdateRepositoryMemberRequest>,
) -> Result<Json<RepositoryMemberResponse>, ApiError> {
    let user = require_scope_user(&state, &headers).await?;
    let now = unix_now()?;
    let member = state.metadata.update_repository_member_permissions(
        &owner,
        &repo_name,
        &user.id,
        &member_user_id,
        input.permissions,
        now,
    )?;
    let repo = find_repo(&state, &owner, &repo_name)?;
    state.publish_repo_change(
        &repo.record.id,
        repo.record.change_version,
        "member-permissions-changed",
    );
    Ok(Json(member_response_for_user(&state, &member)?))
}

pub(crate) async fn delete_repository_member(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((owner, repo_name, member_user_id)): Path<(String, String, String)>,
) -> Result<Json<RepositoryMemberResponse>, ApiError> {
    let user = require_scope_user(&state, &headers).await?;
    let member =
        state
            .metadata
            .remove_repository_member(&owner, &repo_name, &user.id, &member_user_id)?;
    let repo = find_repo(&state, &owner, &repo_name)?;
    state.publish_repo_change(
        &repo.record.id,
        repo.record.change_version,
        "member-removed",
    );
    Ok(Json(member_response_for_user(&state, &member)?))
}

pub(crate) async fn get_repository_invite(
    State(state): State<AppState>,
    Path(token): Path<String>,
) -> Result<Json<RepositoryInviteLookupResponse>, ApiError> {
    let now = unix_now()?;
    let token_hash = repository_invite_token_hash(&token);
    let (repo, invite) = state
        .metadata
        .repository_invite_by_token_hash(&token_hash)?;
    ensure_invite_can_be_used(&invite, now)?;
    Ok(Json(RepositoryInviteLookupResponse {
        repo_id: repo.record.id,
        owner_handle: repo.record.owner_handle,
        repo_name: repo.record.name,
        invited_email: invite.invited_email,
        permissions: invite.permissions,
        expires_at_unix: invite.expires_at_unix,
    }))
}

pub(crate) async fn accept_repository_invite(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(token): Path<String>,
) -> Result<Json<AcceptRepositoryInviteResponse>, ApiError> {
    let user = require_scope_user(&state, &headers).await?;
    let now = unix_now()?;
    let token_hash = repository_invite_token_hash(&token);
    let (repo, member) = state
        .metadata
        .accept_repository_invite(&token_hash, user.clone(), now)?;
    state.publish_repo_change(&repo.record.id, repo.record.change_version, "member-added");
    let summary = repo_summary_for_user(&repo, &user.id)
        .ok_or_else(|| ApiError::internal_message("accepted invite member cannot read repo"))?;
    Ok(Json(AcceptRepositoryInviteResponse {
        repo: summary,
        member: repository_member_response(&member, &user),
    }))
}

fn ensure_invite_can_be_used(
    invite: &crate::domain::store::RepositoryInvite,
    now_unix: u64,
) -> Result<(), ApiError> {
    if invite.state != crate::domain::store::RepositoryInviteState::Pending {
        return Err(ApiError::conflict("repository invite is no longer pending"));
    }
    if now_unix >= invite.expires_at_unix {
        return Err(ApiError::conflict("repository invite expired"));
    }
    Ok(())
}

fn member_response_for_user(
    state: &AppState,
    member: &crate::domain::store::RepositoryMember,
) -> Result<RepositoryMemberResponse, ApiError> {
    state.metadata.read({
        let member = member.clone();
        move |catalog| {
            let user = catalog
                .users
                .get(&member.user_id)
                .ok_or_else(|| ApiError::internal_message("repository member user is missing"))?;
            Ok(repository_member_response(&member, user))
        }
    })
}
