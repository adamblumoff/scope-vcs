use crate::{
    auth::{
        scope::{principal_for_user_id, require_scope_user},
        tokens::{generate_repository_invite_token, repository_invite_token_hash},
    },
    error::ApiError,
    http::{origins::public_app_origin, responses::*},
    persistence::unix_now,
    repo_access::{ensure_repo_read, find_repo},
    repo_events::RepoChangeReason,
    state::AppState,
};
use axum::{
    Json,
    extract::{Path, State},
    http::HeaderMap,
};
use scope_domain::{
    account::UserAccount,
    repository::Repository,
    repository::access::RepositoryAccess,
    repository::collaboration::{RepositoryInvite, RepositoryInviteState, RepositoryMember},
    requests::{Request, RequestViewer, request_policy},
};

pub(crate) async fn list_repository_collaboration(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((owner, repo_name)): Path<(String, String)>,
) -> Result<Json<RepositoryCollaborationResponse>, ApiError> {
    let user = require_scope_user(&state, &headers).await?;
    let repo = find_repo(&state, &owner, &repo_name).await?;
    ensure_collaboration_owner_access(&state, &repo, &user.id)?;
    let (repo, users) = state
        .metadata
        .repositories()
        .repository_collaboration(&owner, &repo_name)
        .await?
        .ok_or_else(|| ApiError::not_found(format!("repo {owner}/{repo_name} not found")))?;

    Ok(Json(repository_collaboration_response(&repo, &users)))
}

pub(crate) async fn create_repository_invite(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((owner, repo_name)): Path<(String, String)>,
    Json(input): Json<CreateRepositoryInviteRequest>,
) -> Result<Json<CreateRepositoryInviteResponse>, ApiError> {
    let metadata = state.metadata.clone();
    let mutation_owner = owner.clone();
    let mutation_repo_name = repo_name.clone();
    let response = mutate_owned_collaboration(
        &state,
        &headers,
        &owner,
        &repo_name,
        RepoChangeReason::InviteUpdated,
        |user| async move {
            let app_origin = public_app_origin("building repository invite URL")?;
            let (secret, token_hash) = generate_repository_invite_token()?;
            let now = unix_now()?;
            let invite_id = format!("repo_invite_{}", token_hash.replace([':', '/'], "_"));
            let invite = metadata
                .repositories()
                .create_repository_invite(
                    scope_postgres::db::CreateRepositoryInviteMutation {
                        owner: mutation_owner,
                        name: mutation_repo_name,
                        owner_user: user.clone(),
                        invited_email: input.email,
                        permissions: input.permissions.into(),
                        invite_id,
                        token_hash,
                        now_unix: now,
                    },
                    &crate::persistence_ids::generate_persistence_id,
                )
                .await?;
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
    let metadata = state.metadata.clone();
    let mutation_owner = owner.clone();
    let mutation_repo_name = repo_name.clone();
    let now_unix = unix_now()?;
    let member = mutate_owned_collaboration(
        &state,
        &headers,
        &owner,
        &repo_name,
        RepoChangeReason::MemberPermissionsChanged,
        |user| async move {
            metadata
                .repositories()
                .update_repository_member_permissions(
                    scope_postgres::db::UpdateRepositoryMemberPermissionsCommand {
                        owner: mutation_owner,
                        name: mutation_repo_name,
                        owner_user_id: user.id,
                        member_user_id,
                        permissions: input.permissions.into(),
                        now_unix,
                    },
                    &crate::persistence_ids::generate_persistence_id,
                )
                .await
                .map_err(Into::into)
        },
    )
    .await?;

    Ok(Json(member_response_for_user(&state, &member).await?))
}

pub(crate) async fn delete_repository_invite(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((owner, repo_name, invite_id)): Path<(String, String, String)>,
) -> Result<Json<RepositoryInviteResponse>, ApiError> {
    let metadata = state.metadata.clone();
    let mutation_owner = owner.clone();
    let mutation_repo_name = repo_name.clone();
    let invite = mutate_owned_collaboration(
        &state,
        &headers,
        &owner,
        &repo_name,
        RepoChangeReason::InviteRevoked,
        |user| async move {
            metadata
                .repositories()
                .revoke_repository_invite(
                    &mutation_owner,
                    &mutation_repo_name,
                    &user.id,
                    &invite_id,
                    unix_now()?,
                    &crate::persistence_ids::generate_persistence_id,
                )
                .await
                .map_err(Into::into)
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
    let metadata = state.metadata.clone();
    let mutation_owner = owner.clone();
    let mutation_repo_name = repo_name.clone();
    let now_unix = unix_now()?;
    let member = mutate_owned_collaboration(
        &state,
        &headers,
        &owner,
        &repo_name,
        RepoChangeReason::MemberRemoved,
        |user| async move {
            metadata
                .repositories()
                .remove_repository_member(
                    &mutation_owner,
                    &mutation_repo_name,
                    &user.id,
                    &member_user_id,
                    now_unix,
                    &crate::persistence_ids::generate_persistence_id,
                )
                .await
                .map_err(Into::into)
        },
    )
    .await?;

    Ok(Json(member_response_for_user(&state, &member).await?))
}

pub(crate) async fn get_repository_invite(
    State(state): State<AppState>,
    Path(token): Path<String>,
) -> Result<Json<RepositoryInviteLookupResponse>, ApiError> {
    let now = unix_now()?;
    let token_hash = repository_invite_token_hash(&token);
    let (repo, invite) = state
        .metadata
        .repositories()
        .repository_invite_by_token_hash(&token_hash)
        .await?;
    ensure_invite_can_be_used(&invite, now)?;
    Ok(Json(RepositoryInviteLookupResponse {
        repo_id: repo.record.id,
        owner_handle: repo.record.owner_handle,
        repo_name: repo.record.name,
        invited_email: invite.invited_email,
        permissions: invite.permissions.into(),
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
        .repositories()
        .accept_repository_invite(
            &token_hash,
            user.clone(),
            now,
            &crate::persistence_ids::generate_persistence_id,
        )
        .await?;
    state
        .publish_repo_change(
            &repo.record.id,
            repo.record.change_version,
            RepoChangeReason::MemberAdded,
        )
        .await;
    let open_request_count =
        open_request_count_for_access(&state, &repo, repo.access_for_user_id(&user.id)).await?;
    let git_origin = crate::http::origins::public_git_origin()?;
    let summary = repo_summary_for_user(&repo, &user.id, open_request_count, &git_origin)
        .ok_or_else(|| ApiError::internal_message("accepted invite member cannot read repo"))?;
    Ok(Json(AcceptRepositoryInviteResponse {
        repo: summary,
        member: repository_member_response(&member, &user),
    }))
}

async fn mutate_owned_collaboration<T, F, Fut>(
    state: &AppState,
    headers: &HeaderMap,
    owner: &str,
    repo_name: &str,
    event: RepoChangeReason,
    mutate: F,
) -> Result<T, ApiError>
where
    F: FnOnce(UserAccount) -> Fut,
    Fut: std::future::Future<Output = Result<T, ApiError>>,
{
    let user = require_scope_user(state, headers).await?;
    let repo = find_repo(state, owner, repo_name).await?;
    ensure_collaboration_owner_access(state, &repo, &user.id)?;
    let result = mutate(user).await?;
    publish_collaboration_change(state, owner, repo_name, event).await?;
    Ok(result)
}

async fn publish_collaboration_change(
    state: &AppState,
    owner: &str,
    repo_name: &str,
    event: RepoChangeReason,
) -> Result<(), ApiError> {
    let repo = find_repo(state, owner, repo_name).await?;
    state
        .publish_repo_change(&repo.record.id, repo.record.change_version, event)
        .await;
    Ok(())
}

fn ensure_collaboration_owner_access(
    state: &AppState,
    repo: &Repository,
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

async fn open_request_count_for_access(
    state: &AppState,
    repo: &Repository,
    access: RepositoryAccess,
) -> Result<usize, ApiError> {
    Ok(state
        .metadata
        .requests()
        .requests_by_repo_id(&repo.record.id)
        .await?
        .into_iter()
        .filter(|request| request_counts_for_access(request, access))
        .count())
}

fn request_counts_for_access(request: &Request, access: RepositoryAccess) -> bool {
    request_policy(request, RequestViewer::new(access, None, false)).counts_as_open
}

async fn member_response_for_user(
    state: &AppState,
    member: &RepositoryMember,
) -> Result<RepositoryMemberResponse, ApiError> {
    let user = state.metadata.repositories().user(&member.user_id).await?;
    Ok(repository_member_response(member, &user))
}
