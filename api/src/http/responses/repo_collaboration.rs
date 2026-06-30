use super::RepoSummaryResponse;
use crate::domain::store::{
    RepositoryInvite, RepositoryInviteState, RepositoryMember, RepositoryMemberPermissions,
    StoredRepository, UserAccount,
};
use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub(crate) struct RepositoryCollaborationResponse {
    pub(crate) members: Vec<RepositoryMemberResponse>,
    pub(crate) invites: Vec<RepositoryInviteResponse>,
}

#[derive(Debug, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub(crate) struct RepositoryMemberResponse {
    pub(crate) user_id: String,
    pub(crate) handle: String,
    pub(crate) email: String,
    pub(crate) permissions: RepositoryMemberPermissions,
    pub(crate) created_at_unix: u64,
    pub(crate) updated_at_unix: u64,
}

#[derive(Debug, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub(crate) struct RepositoryInviteResponse {
    pub(crate) id: String,
    pub(crate) invited_email: String,
    pub(crate) permissions: RepositoryMemberPermissions,
    pub(crate) state: RepositoryInviteState,
    pub(crate) expires_at_unix: u64,
}

#[derive(Debug, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub(crate) struct CreateRepositoryInviteRequest {
    pub(crate) email: String,
    pub(crate) permissions: RepositoryMemberPermissions,
}

#[derive(Debug, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub(crate) struct CreateRepositoryInviteResponse {
    pub(crate) invite: RepositoryInviteResponse,
    pub(crate) invite_url: String,
}

#[derive(Debug, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub(crate) struct UpdateRepositoryMemberRequest {
    pub(crate) permissions: RepositoryMemberPermissions,
}

#[derive(Debug, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub(crate) struct RepositoryInviteLookupResponse {
    pub(crate) repo_id: String,
    pub(crate) owner_handle: String,
    pub(crate) repo_name: String,
    pub(crate) invited_email: String,
    pub(crate) permissions: RepositoryMemberPermissions,
    pub(crate) expires_at_unix: u64,
}

#[derive(Debug, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub(crate) struct AcceptRepositoryInviteResponse {
    pub(crate) repo: RepoSummaryResponse,
    pub(crate) member: RepositoryMemberResponse,
}

pub(crate) fn repository_collaboration_response(
    repo: &StoredRepository,
    users: &std::collections::BTreeMap<String, UserAccount>,
) -> RepositoryCollaborationResponse {
    let mut members = repo
        .members
        .iter()
        .filter_map(|member| {
            users
                .get(&member.user_id)
                .map(|user| repository_member_response(member, user))
        })
        .collect::<Vec<_>>();
    members.sort_by(|left, right| {
        left.email
            .cmp(&right.email)
            .then(left.user_id.cmp(&right.user_id))
    });

    let mut invites = repo
        .invitations
        .iter()
        .map(repository_invite_response)
        .collect::<Vec<_>>();
    invites.sort_by(|left, right| {
        left.invited_email
            .cmp(&right.invited_email)
            .then(left.id.cmp(&right.id))
    });

    RepositoryCollaborationResponse { members, invites }
}

pub(crate) fn repository_member_response(
    member: &RepositoryMember,
    user: &UserAccount,
) -> RepositoryMemberResponse {
    RepositoryMemberResponse {
        user_id: member.user_id.clone(),
        handle: user.handle.clone(),
        email: user.email.clone(),
        permissions: member.permissions,
        created_at_unix: member.created_at_unix,
        updated_at_unix: member.updated_at_unix,
    }
}

pub(crate) fn repository_invite_response(invite: &RepositoryInvite) -> RepositoryInviteResponse {
    RepositoryInviteResponse {
        id: invite.id.clone(),
        invited_email: invite.invited_email.clone(),
        permissions: invite.permissions,
        state: invite.state,
        expires_at_unix: invite.expires_at_unix,
    }
}
