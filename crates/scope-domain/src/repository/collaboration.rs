use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RepositoryMemberPermissions {
    pub can_push: bool,
    pub can_change_file_visibility: bool,
    pub can_apply_changes: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RepositoryMember {
    pub repo_id: String,
    pub user_id: String,
    pub permissions: RepositoryMemberPermissions,
    pub created_at_unix: u64,
    pub updated_at_unix: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum RepositoryInviteState {
    Pending,
    Accepted,
    Revoked,
    Expired,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RepositoryInvite {
    pub id: String,
    pub repo_id: String,
    pub invited_email: String,
    pub invited_email_normalized: String,
    pub permissions: RepositoryMemberPermissions,
    pub invited_by_user_id: String,
    pub state: RepositoryInviteState,
    pub token_hash: String,
    pub created_at_unix: u64,
    pub updated_at_unix: u64,
    pub expires_at_unix: u64,
    pub accepted_by_user_id: Option<String>,
    pub accepted_at_unix: Option<u64>,
    pub revoked_at_unix: Option<u64>,
}

pub fn normalize_repository_invite_email(email: &str) -> String {
    email.trim().to_ascii_lowercase()
}

pub fn repository_member_sort_key(member: &RepositoryMember) -> (&str, &str) {
    (&member.repo_id, &member.user_id)
}

pub fn repository_invite_sort_key(invite: &RepositoryInvite) -> (&str, &str, &str) {
    (
        &invite.repo_id,
        &invite.invited_email_normalized,
        &invite.id,
    )
}
