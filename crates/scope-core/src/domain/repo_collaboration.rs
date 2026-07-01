use super::store::{
    RepositoryInvite, RepositoryInviteState, RepositoryMember, RepositoryMemberPermissions,
    StoredRepository, UserAccount, normalize_repository_invite_email,
};
use crate::error::ApiError;

pub const REPOSITORY_INVITE_TTL_SECS: u64 = 7 * 24 * 60 * 60;

pub enum AcceptRepositoryInviteOutcome {
    Accepted(RepositoryMember),
    Expired,
}

pub struct CreateRepositoryInviteCommand<'a> {
    pub id: String,
    pub owner: &'a UserAccount,
    pub invited_email: String,
    pub invitee: Option<&'a UserAccount>,
    pub permissions: RepositoryMemberPermissions,
    pub token_hash: String,
    pub now_unix: u64,
}

pub fn create_or_refresh_repository_invite(
    repo: &mut StoredRepository,
    command: CreateRepositoryInviteCommand<'_>,
) -> Result<RepositoryInvite, ApiError> {
    ensure_can_manage_members(repo, &command.owner.id)?;
    let normalized = validate_invite_email(&command.invited_email)?;
    if normalize_repository_invite_email(&command.owner.email) == normalized {
        return Err(ApiError::conflict("repository owner cannot be invited"));
    }
    if let Some(invitee) = command.invitee
        && (repo.is_owner_user(&invitee.id) || repo.member_for_user(&invitee.id).is_some())
    {
        return Err(ApiError::conflict("user is already a repository member"));
    }

    if let Some(index) = repo.invitations.iter().position(|invite| {
        invite.state == RepositoryInviteState::Pending
            && invite.invited_email_normalized == normalized
    }) {
        let existing = &mut repo.invitations[index];
        existing.invited_email = command.invited_email.trim().to_string();
        existing.permissions = command.permissions;
        existing.invited_by_user_id = command.owner.id.clone();
        existing.token_hash = command.token_hash;
        existing.updated_at_unix = command.now_unix;
        existing.expires_at_unix = command.now_unix + REPOSITORY_INVITE_TTL_SECS;
        let refreshed = existing.clone();
        repo.bump_change_version();
        return Ok(refreshed);
    }

    let invite = RepositoryInvite {
        id: command.id,
        repo_id: repo.record.id.clone(),
        invited_email: command.invited_email.trim().to_string(),
        invited_email_normalized: normalized,
        permissions: command.permissions,
        invited_by_user_id: command.owner.id.clone(),
        state: RepositoryInviteState::Pending,
        token_hash: command.token_hash,
        created_at_unix: command.now_unix,
        updated_at_unix: command.now_unix,
        expires_at_unix: command.now_unix + REPOSITORY_INVITE_TTL_SECS,
        accepted_by_user_id: None,
        accepted_at_unix: None,
        revoked_at_unix: None,
    };
    repo.invitations.push(invite.clone());
    sort_invitations(repo);
    repo.bump_change_version();
    Ok(invite)
}

pub fn accept_repository_invite(
    repo: &mut StoredRepository,
    user: &UserAccount,
    token_hash: &str,
    now_unix: u64,
) -> Result<AcceptRepositoryInviteOutcome, ApiError> {
    let normalized_user_email = normalize_repository_invite_email(&user.email);
    if repo.is_owner_user(&user.id) || repo.member_for_user(&user.id).is_some() {
        return Err(ApiError::conflict("user is already a repository member"));
    }
    let invite = repo
        .invitations
        .iter_mut()
        .find(|invite| invite.token_hash == token_hash)
        .ok_or_else(|| ApiError::not_found("repository invite not found"))?;
    if invite.state != RepositoryInviteState::Pending {
        return Err(ApiError::conflict("repository invite is no longer pending"));
    }
    if now_unix >= invite.expires_at_unix {
        invite.state = RepositoryInviteState::Expired;
        invite.updated_at_unix = now_unix;
        repo.bump_change_version();
        return Ok(AcceptRepositoryInviteOutcome::Expired);
    }
    if !user.email_verified || normalized_user_email != invite.invited_email_normalized {
        return Err(ApiError::forbidden(
            "sign in with the verified invited email to accept this invite",
        ));
    }
    invite.state = RepositoryInviteState::Accepted;
    invite.accepted_by_user_id = Some(user.id.clone());
    invite.accepted_at_unix = Some(now_unix);
    invite.updated_at_unix = now_unix;
    let member = RepositoryMember {
        repo_id: repo.record.id.clone(),
        user_id: user.id.clone(),
        permissions: invite.permissions,
        created_at_unix: now_unix,
        updated_at_unix: now_unix,
    };
    repo.members.push(member.clone());
    sort_members(repo);
    repo.bump_change_version();
    Ok(AcceptRepositoryInviteOutcome::Accepted(member))
}

pub fn revoke_repository_invite(
    repo: &mut StoredRepository,
    owner_user_id: &str,
    invite_id: &str,
    now_unix: u64,
) -> Result<RepositoryInvite, ApiError> {
    ensure_can_manage_members(repo, owner_user_id)?;
    let invite = repo
        .invitations
        .iter_mut()
        .find(|invite| invite.id == invite_id)
        .ok_or_else(|| ApiError::not_found("repository invite not found"))?;
    if invite.state != RepositoryInviteState::Pending {
        return Err(ApiError::conflict("repository invite is no longer pending"));
    }

    invite.state = RepositoryInviteState::Revoked;
    invite.revoked_at_unix = Some(now_unix);
    invite.updated_at_unix = now_unix;
    let invite = invite.clone();
    repo.bump_change_version();
    Ok(invite)
}

pub fn update_repository_member_permissions(
    repo: &mut StoredRepository,
    owner_user_id: &str,
    member_user_id: &str,
    permissions: RepositoryMemberPermissions,
    now_unix: u64,
) -> Result<RepositoryMember, ApiError> {
    ensure_can_manage_members(repo, owner_user_id)?;
    let member = repo
        .members
        .iter_mut()
        .find(|member| member.user_id == member_user_id)
        .ok_or_else(|| ApiError::not_found("repository member not found"))?;
    member.permissions = permissions;
    member.updated_at_unix = now_unix;
    let member = member.clone();
    repo.bump_change_version();
    Ok(member)
}

pub fn remove_repository_member(
    repo: &mut StoredRepository,
    owner_user_id: &str,
    member_user_id: &str,
) -> Result<RepositoryMember, ApiError> {
    ensure_can_manage_members(repo, owner_user_id)?;
    let index = repo
        .members
        .iter()
        .position(|member| member.user_id == member_user_id)
        .ok_or_else(|| ApiError::not_found("repository member not found"))?;
    let removed = repo.members.remove(index);
    repo.bump_change_version();
    Ok(removed)
}

pub fn ensure_can_manage_members(repo: &StoredRepository, user_id: &str) -> Result<(), ApiError> {
    if repo.access_for_user_id(user_id).can_manage_members {
        Ok(())
    } else if repo.is_owner_user(user_id) {
        Err(ApiError::conflict(
            "repository must be published before inviting members",
        ))
    } else {
        Err(ApiError::forbidden("owner role required"))
    }
}

fn validate_invite_email(email: &str) -> Result<String, ApiError> {
    let normalized = normalize_repository_invite_email(email);
    if normalized.is_empty() || !normalized.contains('@') {
        return Err(ApiError::bad_request("valid invited email is required"));
    }
    Ok(normalized)
}

fn sort_members(repo: &mut StoredRepository) {
    repo.members.sort_by(|left, right| {
        left.user_id
            .cmp(&right.user_id)
            .then(left.created_at_unix.cmp(&right.created_at_unix))
    });
}

fn sort_invitations(repo: &mut StoredRepository) {
    repo.invitations.sort_by(|left, right| {
        left.invited_email_normalized
            .cmp(&right.invited_email_normalized)
            .then(left.id.cmp(&right.id))
    });
}
