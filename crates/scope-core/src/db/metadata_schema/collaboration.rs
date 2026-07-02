use super::*;

#[derive(Copy, Clone)]
pub enum RepositoryMembers {
    Table,
    RepoId,
    UserId,
    Permissions,
    CreatedAtUnix,
    UpdatedAtUnix,
}

impl_iden!(RepositoryMembers {
    Table => "scope_repository_members",
    RepoId => "repo_id",
    UserId => "user_id",
    Permissions => "permissions",
    CreatedAtUnix => "created_at_unix",
    UpdatedAtUnix => "updated_at_unix",
});

#[derive(Copy, Clone)]
pub enum RepositoryInvites {
    Table,
    Id,
    RepoId,
    InvitedEmail,
    InvitedEmailNormalized,
    Permissions,
    InvitedByUserId,
    State,
    TokenHash,
    CreatedAtUnix,
    UpdatedAtUnix,
    ExpiresAtUnix,
    AcceptedByUserId,
    AcceptedAtUnix,
    RevokedAtUnix,
}

impl_iden!(RepositoryInvites {
    Table => "scope_repository_invites",
    Id => "id",
    RepoId => "repo_id",
    InvitedEmail => "invited_email",
    InvitedEmailNormalized => "invited_email_normalized",
    Permissions => "permissions",
    InvitedByUserId => "invited_by_user_id",
    State => "state",
    TokenHash => "token_hash",
    CreatedAtUnix => "created_at_unix",
    UpdatedAtUnix => "updated_at_unix",
    ExpiresAtUnix => "expires_at_unix",
    AcceptedByUserId => "accepted_by_user_id",
    AcceptedAtUnix => "accepted_at_unix",
    RevokedAtUnix => "revoked_at_unix",
});

const REPOSITORY_MEMBER_COLUMNS: &[&str] = &[
    RepositoryMembers::RepoId.as_str(),
    RepositoryMembers::UserId.as_str(),
    RepositoryMembers::Permissions.as_str(),
    RepositoryMembers::CreatedAtUnix.as_str(),
    RepositoryMembers::UpdatedAtUnix.as_str(),
];

const REPOSITORY_INVITE_COLUMNS: &[&str] = &[
    RepositoryInvites::Id.as_str(),
    RepositoryInvites::RepoId.as_str(),
    RepositoryInvites::InvitedEmail.as_str(),
    RepositoryInvites::InvitedEmailNormalized.as_str(),
    RepositoryInvites::Permissions.as_str(),
    RepositoryInvites::InvitedByUserId.as_str(),
    RepositoryInvites::State.as_str(),
    RepositoryInvites::TokenHash.as_str(),
    RepositoryInvites::CreatedAtUnix.as_str(),
    RepositoryInvites::UpdatedAtUnix.as_str(),
    RepositoryInvites::ExpiresAtUnix.as_str(),
    RepositoryInvites::AcceptedByUserId.as_str(),
    RepositoryInvites::AcceptedAtUnix.as_str(),
    RepositoryInvites::RevokedAtUnix.as_str(),
];

pub const TABLES: &[MetadataTableSpec] = &[
    MetadataTableSpec {
        table: RepositoryMembers::Table.as_str(),
        columns: REPOSITORY_MEMBER_COLUMNS,
        counts_for_catalog_rows: true,
    },
    MetadataTableSpec {
        table: RepositoryInvites::Table.as_str(),
        columns: REPOSITORY_INVITE_COLUMNS,
        counts_for_catalog_rows: true,
    },
];
