use crate::domain::{
    requests::{Request, RequestActorRole, RequestBaseAudience, RequestState},
    store::{
        RepositoryAccess, RepositoryActor, RepositoryInvite, RepositoryInviteState,
        RepositoryMember, StoredRepository, UserAccount,
    },
};
use crate::{
    auth::{
        scope::{principal_for_user_id, require_scope_user},
        tokens::{generate_repository_invite_token, repository_invite_token_hash},
    },
    error::ApiError,
    http::{origins::public_app_origin, responses::*},
    persistence::unix_now,
    state::AppState,
    state::{ensure_repo_read, find_repo},
};
use axum::{
    Json,
    extract::{Path, State},
    http::HeaderMap,
};

pub(crate) async fn list_repository_collaboration(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((owner, repo_name)): Path<(String, String)>,
) -> Result<Json<RepositoryCollaborationResponse>, ApiError> {
    let user = require_scope_user(&state, &headers).await?;
    let repo = find_repo(&state, &owner, &repo_name)?;
    ensure_collaboration_owner_access(&state, &repo, &user.id)?;
    let owner_for_read = owner.clone();
    let repo_for_read = repo_name.clone();
    let response = state.metadata.read(move |catalog| {
        let repo = catalog
            .repository(&owner_for_read, &repo_for_read)
            .ok_or_else(|| {
                ApiError::not_found(format!("repo {owner_for_read}/{repo_for_read} not found"))
            })?;
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
    let response = mutate_owned_collaboration(
        &state,
        &headers,
        &owner,
        &repo_name,
        "invite-updated",
        |user| {
            let app_origin = public_app_origin("building repository invite URL")?;
            let (secret, token_hash) = generate_repository_invite_token()?;
            let now = unix_now()?;
            let invite_id = format!("repo_invite_{}", token_hash.replace([':', '/'], "_"));
            let invite = state.metadata.create_repository_invite(
                crate::db::CreateRepositoryInviteMutation {
                    owner: owner.clone(),
                    name: repo_name.clone(),
                    owner_user: user.clone(),
                    invited_email: input.email,
                    permissions: input.permissions,
                    invite_id,
                    token_hash,
                    now_unix: now,
                },
            )?;
            Ok(CreateRepositoryInviteResponse {
                invite: repository_invite_response(&invite),
                invite_url: format!("{}/invites/{}", app_origin.trim_end_matches('/'), secret),
            })
        },
    )
    .await?;

    Ok(Json(response))
}

pub(crate) async fn update_repository_member(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((owner, repo_name, member_user_id)): Path<(String, String, String)>,
    Json(input): Json<UpdateRepositoryMemberRequest>,
) -> Result<Json<RepositoryMemberResponse>, ApiError> {
    let member = mutate_owned_collaboration(
        &state,
        &headers,
        &owner,
        &repo_name,
        "member-permissions-changed",
        |user| {
            state.metadata.update_repository_member_permissions(
                &owner,
                &repo_name,
                &user.id,
                &member_user_id,
                input.permissions,
                unix_now()?,
            )
        },
    )
    .await?;

    Ok(Json(member_response_for_user(&state, &member)?))
}

pub(crate) async fn delete_repository_invite(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((owner, repo_name, invite_id)): Path<(String, String, String)>,
) -> Result<Json<RepositoryInviteResponse>, ApiError> {
    let invite = mutate_owned_collaboration(
        &state,
        &headers,
        &owner,
        &repo_name,
        "invite-revoked",
        |user| {
            state.metadata.revoke_repository_invite(
                &owner,
                &repo_name,
                &user.id,
                &invite_id,
                unix_now()?,
            )
        },
    )
    .await?;

    Ok(Json(repository_invite_response(&invite)))
}

pub(crate) async fn delete_repository_member(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((owner, repo_name, member_user_id)): Path<(String, String, String)>,
) -> Result<Json<RepositoryMemberResponse>, ApiError> {
    let member = mutate_owned_collaboration(
        &state,
        &headers,
        &owner,
        &repo_name,
        "member-removed",
        |user| {
            state
                .metadata
                .remove_repository_member(&owner, &repo_name, &user.id, &member_user_id)
        },
    )
    .await?;

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
    let open_request_count =
        open_request_count_for_access(&state, &repo, repo.access_for_user_id(&user.id))?;
    let summary = repo_summary_for_user(&repo, &user.id, open_request_count)
        .ok_or_else(|| ApiError::internal_message("accepted invite member cannot read repo"))?;
    Ok(Json(AcceptRepositoryInviteResponse {
        repo: summary,
        member: repository_member_response(&member, &user),
    }))
}

async fn mutate_owned_collaboration<T>(
    state: &AppState,
    headers: &HeaderMap,
    owner: &str,
    repo_name: &str,
    event: &'static str,
    mutate: impl FnOnce(&UserAccount) -> Result<T, ApiError>,
) -> Result<T, ApiError> {
    let user = require_scope_user(state, headers).await?;
    let repo = find_repo(state, owner, repo_name)?;
    ensure_collaboration_owner_access(state, &repo, &user.id)?;
    let result = mutate(&user)?;
    publish_collaboration_change(state, owner, repo_name, event)?;
    Ok(result)
}

fn publish_collaboration_change(
    state: &AppState,
    owner: &str,
    repo_name: &str,
    event: &'static str,
) -> Result<(), ApiError> {
    let repo = find_repo(state, owner, repo_name)?;
    state.publish_repo_change(&repo.record.id, repo.record.change_version, event);
    Ok(())
}

fn ensure_collaboration_owner_access(
    state: &AppState,
    repo: &StoredRepository,
    user_id: &str,
) -> Result<(), ApiError> {
    let principal = principal_for_user_id(repo, user_id);
    ensure_repo_read(state, repo, &principal)?;
    if repo.is_owner_user(user_id) {
        Ok(())
    } else {
        Err(ApiError::forbidden("owner role required"))
    }
}

fn ensure_invite_can_be_used(invite: &RepositoryInvite, now_unix: u64) -> Result<(), ApiError> {
    if invite.state != RepositoryInviteState::Pending {
        return Err(ApiError::conflict("repository invite is no longer pending"));
    }
    if now_unix >= invite.expires_at_unix {
        return Err(ApiError::conflict("repository invite expired"));
    }
    Ok(())
}

fn open_request_count_for_access(
    state: &AppState,
    repo: &StoredRepository,
    access: RepositoryAccess,
) -> Result<usize, ApiError> {
    Ok(state
        .metadata
        .requests_by_repo_id(&repo.record.id)?
        .into_iter()
        .filter(|request| request_counts_for_access(request, access))
        .count())
}

fn request_counts_for_access(request: &Request, access: RepositoryAccess) -> bool {
    if matches!(
        request.state,
        RequestState::Working | RequestState::Resolved | RequestState::Withdrawn
    ) {
        return false;
    }
    access.actor != RepositoryActor::Public
        || (request.author_role == RequestActorRole::Public
            && request.base_audience == RequestBaseAudience::Public)
}

fn member_response_for_user(
    state: &AppState,
    member: &RepositoryMember,
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
