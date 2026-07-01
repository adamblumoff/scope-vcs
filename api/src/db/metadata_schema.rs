use sea_orm_migration::prelude::Iden;

#[derive(Copy, Clone)]
pub(super) struct MetadataTableSpec {
    pub(super) table: &'static str,
    pub(super) columns: &'static [&'static str],
    pub(super) counts_for_catalog_rows: bool,
}

macro_rules! impl_iden {
    ($name:ident { $($variant:ident => $value:literal),+ $(,)? }) => {
        impl $name {
            pub(super) const fn as_str(self) -> &'static str {
                match self {
                    $(Self::$variant => $value,)+
                }
            }
        }

        impl Iden for $name {
            fn unquoted(&self, s: &mut dyn std::fmt::Write) {
                std::fmt::Write::write_str(s, self.as_str())
                    .expect("writing identifier cannot fail");
            }
        }
    };
}

#[derive(Copy, Clone)]
pub(super) enum Users {
    Table,
    Id,
    Handle,
    Email,
    EmailVerified,
    Access,
}

impl_iden!(Users {
    Table => "scope_users",
    Id => "id",
    Handle => "handle",
    Email => "email",
    EmailVerified => "email_verified",
    Access => "access",
});

#[derive(Copy, Clone)]
pub(super) enum AuthIdentities {
    Table,
    Provider,
    Subject,
    UserId,
}

impl_iden!(AuthIdentities {
    Table => "scope_auth_identities",
    Provider => "provider",
    Subject => "subject",
    UserId => "user_id",
});

#[derive(Copy, Clone)]
pub(super) enum Repositories {
    Table,
    Id,
    OwnerHandle,
    Name,
    OwnerUserId,
    PublicationState,
    DefaultVisibility,
    ChangeVersion,
    PendingImport,
    Policy,
    Graph,
    VisibilityEvents,
    StagedUpdate,
}

impl_iden!(Repositories {
    Table => "scope_repositories",
    Id => "id",
    OwnerHandle => "owner_handle",
    Name => "name",
    OwnerUserId => "owner_user_id",
    PublicationState => "publication_state",
    DefaultVisibility => "default_visibility",
    ChangeVersion => "change_version",
    PendingImport => "pending_import",
    Policy => "policy",
    Graph => "graph",
    VisibilityEvents => "visibility_events",
    StagedUpdate => "staged_update",
});

#[derive(Copy, Clone)]
pub(super) enum RepositorySettings {
    Table,
    RepoId,
    IncludeIgnoredFiles,
    ReviewPushesBeforeApplying,
}

impl_iden!(RepositorySettings {
    Table => "scope_repository_settings",
    RepoId => "repo_id",
    IncludeIgnoredFiles => "include_ignored_files",
    ReviewPushesBeforeApplying => "review_pushes_before_applying",
});

#[derive(Copy, Clone)]
pub(super) enum RepositoryFirstPushTokens {
    Table,
    RepoId,
    TokenHash,
    OwnerUserId,
    CreatedAtUnix,
    ExpiresAtUnix,
    UsedAtUnix,
}

impl_iden!(RepositoryFirstPushTokens {
    Table => "scope_repository_first_push_tokens",
    RepoId => "repo_id",
    TokenHash => "token_hash",
    OwnerUserId => "owner_user_id",
    CreatedAtUnix => "created_at_unix",
    ExpiresAtUnix => "expires_at_unix",
    UsedAtUnix => "used_at_unix",
});

#[derive(Copy, Clone)]
pub(super) enum RepositoryGitPushTokens {
    Table,
    RepoId,
    TokenHash,
    OwnerUserId,
    CreatedAtUnix,
}

impl_iden!(RepositoryGitPushTokens {
    Table => "scope_repository_git_push_tokens",
    RepoId => "repo_id",
    TokenHash => "token_hash",
    OwnerUserId => "owner_user_id",
    CreatedAtUnix => "created_at_unix",
});

#[derive(Copy, Clone)]
pub(super) enum RepositoryGitCloneTokens {
    Table,
    RepoId,
    TokenHash,
    UserId,
    CreatedAtUnix,
}

impl_iden!(RepositoryGitCloneTokens {
    Table => "scope_repository_git_clone_tokens",
    RepoId => "repo_id",
    TokenHash => "token_hash",
    UserId => "user_id",
    CreatedAtUnix => "created_at_unix",
});

#[derive(Copy, Clone)]
pub(super) enum RepositoryGitSnapshots {
    Table,
    RepoId,
    ObjectKey,
    Sha256,
    GitOid,
    SizeBytes,
    LineCount,
}

impl_iden!(RepositoryGitSnapshots {
    Table => "scope_repository_git_snapshots",
    RepoId => "repo_id",
    ObjectKey => "object_key",
    Sha256 => "sha256",
    GitOid => "git_oid",
    SizeBytes => "size_bytes",
    LineCount => "line_count",
});

#[derive(Copy, Clone)]
pub(super) enum CliDeviceLogins {
    Table,
    DeviceCodeHash,
    UserCodeHash,
    CreatedAtUnix,
    ExpiresAtUnix,
    CompletedUserId,
    CompletedAtUnix,
    ConsumedAtUnix,
}

impl_iden!(CliDeviceLogins {
    Table => "scope_cli_device_logins",
    DeviceCodeHash => "device_code_hash",
    UserCodeHash => "user_code_hash",
    CreatedAtUnix => "created_at_unix",
    ExpiresAtUnix => "expires_at_unix",
    CompletedUserId => "completed_user_id",
    CompletedAtUnix => "completed_at_unix",
    ConsumedAtUnix => "consumed_at_unix",
});

#[derive(Copy, Clone)]
pub(super) enum CliBrowserLogins {
    Table,
    RequestId,
    RequestSecretHash,
    CallbackUrl,
    CallbackCodeHash,
    CreatedAtUnix,
    ExpiresAtUnix,
    CompletedUserId,
    CompletedAtUnix,
    ConsumedAtUnix,
}

impl_iden!(CliBrowserLogins {
    Table => "scope_cli_browser_logins",
    RequestId => "request_id",
    RequestSecretHash => "request_secret_hash",
    CallbackUrl => "callback_url",
    CallbackCodeHash => "callback_code_hash",
    CreatedAtUnix => "created_at_unix",
    ExpiresAtUnix => "expires_at_unix",
    CompletedUserId => "completed_user_id",
    CompletedAtUnix => "completed_at_unix",
    ConsumedAtUnix => "consumed_at_unix",
});

#[derive(Copy, Clone)]
pub(super) enum CliExchangeGrants {
    Table,
    GrantHash,
    UserId,
    CreatedAtUnix,
    ExpiresAtUnix,
    ConsumedAtUnix,
}

impl_iden!(CliExchangeGrants {
    Table => "scope_cli_exchange_grants",
    GrantHash => "grant_hash",
    UserId => "user_id",
    CreatedAtUnix => "created_at_unix",
    ExpiresAtUnix => "expires_at_unix",
    ConsumedAtUnix => "consumed_at_unix",
});

#[derive(Copy, Clone)]
pub(super) enum CliSessions {
    Table,
    Id,
    TokenHash,
    UserId,
    Label,
    CreatedAtUnix,
    LastUsedAtUnix,
    ExpiresAtUnix,
    RevokedAtUnix,
}

impl_iden!(CliSessions {
    Table => "scope_cli_sessions",
    Id => "id",
    TokenHash => "token_hash",
    UserId => "user_id",
    Label => "label",
    CreatedAtUnix => "created_at_unix",
    LastUsedAtUnix => "last_used_at_unix",
    ExpiresAtUnix => "expires_at_unix",
    RevokedAtUnix => "revoked_at_unix",
});

#[derive(Copy, Clone)]
pub(super) enum RepositoryMembers {
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
pub(super) enum RepositoryInvites {
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

#[derive(Copy, Clone)]
pub(super) enum MetadataLocks {
    Table,
    Key,
}

impl_iden!(MetadataLocks {
    Table => "scope_metadata_locks",
    Key => "key",
});

#[derive(Copy, Clone)]
pub(super) enum RepoStorageCleanupJobs {
    Table,
    RepoId,
    Generation,
    OwnerHandle,
    RepoName,
    Attempts,
    NextRunAtUnix,
    LastError,
    CompletedAtUnix,
    CreatedAtUnix,
    UpdatedAtUnix,
}

impl_iden!(RepoStorageCleanupJobs {
    Table => "scope_repo_storage_cleanup_jobs",
    RepoId => "repo_id",
    Generation => "generation",
    OwnerHandle => "owner_handle",
    RepoName => "repo_name",
    Attempts => "attempts",
    NextRunAtUnix => "next_run_at_unix",
    LastError => "last_error",
    CompletedAtUnix => "completed_at_unix",
    CreatedAtUnix => "created_at_unix",
    UpdatedAtUnix => "updated_at_unix",
});

#[derive(Copy, Clone)]
pub(super) enum SourceBlobCleanupJobs {
    Table,
    ObjectKey,
    Generation,
    Sha256,
    GitOid,
    SizeBytes,
    LineCount,
    Attempts,
    NextRunAtUnix,
    LastError,
    CompletedAtUnix,
    CreatedAtUnix,
    UpdatedAtUnix,
}

impl_iden!(SourceBlobCleanupJobs {
    Table => "scope_source_blob_cleanup_jobs",
    ObjectKey => "object_key",
    Generation => "generation",
    Sha256 => "sha256",
    GitOid => "git_oid",
    SizeBytes => "size_bytes",
    LineCount => "line_count",
    Attempts => "attempts",
    NextRunAtUnix => "next_run_at_unix",
    LastError => "last_error",
    CompletedAtUnix => "completed_at_unix",
    CreatedAtUnix => "created_at_unix",
    UpdatedAtUnix => "updated_at_unix",
});

#[derive(Copy, Clone)]
pub(super) enum MetadataResetEvents {
    Table,
    Id,
    ResetAtUnix,
    Trigger,
    Reason,
}

impl_iden!(MetadataResetEvents {
    Table => "scope_metadata_reset_events",
    Id => "id",
    ResetAtUnix => "reset_at_unix",
    Trigger => "trigger",
    Reason => "reason",
});

const METADATA_LOCK_COLUMNS: &[&str] = &[MetadataLocks::Key.as_str()];
const REPO_STORAGE_CLEANUP_JOB_COLUMNS: &[&str] = &[
    RepoStorageCleanupJobs::RepoId.as_str(),
    RepoStorageCleanupJobs::Generation.as_str(),
    RepoStorageCleanupJobs::OwnerHandle.as_str(),
    RepoStorageCleanupJobs::RepoName.as_str(),
    RepoStorageCleanupJobs::Attempts.as_str(),
    RepoStorageCleanupJobs::NextRunAtUnix.as_str(),
    RepoStorageCleanupJobs::LastError.as_str(),
    RepoStorageCleanupJobs::CompletedAtUnix.as_str(),
    RepoStorageCleanupJobs::CreatedAtUnix.as_str(),
    RepoStorageCleanupJobs::UpdatedAtUnix.as_str(),
];
const SOURCE_BLOB_CLEANUP_JOB_COLUMNS: &[&str] = &[
    SourceBlobCleanupJobs::ObjectKey.as_str(),
    SourceBlobCleanupJobs::Generation.as_str(),
    SourceBlobCleanupJobs::Sha256.as_str(),
    SourceBlobCleanupJobs::GitOid.as_str(),
    SourceBlobCleanupJobs::SizeBytes.as_str(),
    SourceBlobCleanupJobs::LineCount.as_str(),
    SourceBlobCleanupJobs::Attempts.as_str(),
    SourceBlobCleanupJobs::NextRunAtUnix.as_str(),
    SourceBlobCleanupJobs::LastError.as_str(),
    SourceBlobCleanupJobs::CompletedAtUnix.as_str(),
    SourceBlobCleanupJobs::CreatedAtUnix.as_str(),
    SourceBlobCleanupJobs::UpdatedAtUnix.as_str(),
];
const USER_COLUMNS: &[&str] = &[
    Users::Id.as_str(),
    Users::Handle.as_str(),
    Users::Email.as_str(),
    Users::EmailVerified.as_str(),
    Users::Access.as_str(),
];
const AUTH_IDENTITY_COLUMNS: &[&str] = &[
    AuthIdentities::Provider.as_str(),
    AuthIdentities::Subject.as_str(),
    AuthIdentities::UserId.as_str(),
];
const REPOSITORY_COLUMNS: &[&str] = &[
    Repositories::Id.as_str(),
    Repositories::OwnerHandle.as_str(),
    Repositories::Name.as_str(),
    Repositories::OwnerUserId.as_str(),
    Repositories::PublicationState.as_str(),
    Repositories::DefaultVisibility.as_str(),
    Repositories::ChangeVersion.as_str(),
    Repositories::PendingImport.as_str(),
    Repositories::Policy.as_str(),
    Repositories::Graph.as_str(),
    Repositories::VisibilityEvents.as_str(),
    Repositories::StagedUpdate.as_str(),
];
const REPOSITORY_SETTING_COLUMNS: &[&str] = &[
    RepositorySettings::RepoId.as_str(),
    RepositorySettings::IncludeIgnoredFiles.as_str(),
    RepositorySettings::ReviewPushesBeforeApplying.as_str(),
];
const REPOSITORY_FIRST_PUSH_TOKEN_COLUMNS: &[&str] = &[
    RepositoryFirstPushTokens::RepoId.as_str(),
    RepositoryFirstPushTokens::TokenHash.as_str(),
    RepositoryFirstPushTokens::OwnerUserId.as_str(),
    RepositoryFirstPushTokens::CreatedAtUnix.as_str(),
    RepositoryFirstPushTokens::ExpiresAtUnix.as_str(),
    RepositoryFirstPushTokens::UsedAtUnix.as_str(),
];
const REPOSITORY_GIT_PUSH_TOKEN_COLUMNS: &[&str] = &[
    RepositoryGitPushTokens::RepoId.as_str(),
    RepositoryGitPushTokens::TokenHash.as_str(),
    RepositoryGitPushTokens::OwnerUserId.as_str(),
    RepositoryGitPushTokens::CreatedAtUnix.as_str(),
];
const REPOSITORY_GIT_CLONE_TOKEN_COLUMNS: &[&str] = &[
    RepositoryGitCloneTokens::RepoId.as_str(),
    RepositoryGitCloneTokens::TokenHash.as_str(),
    RepositoryGitCloneTokens::UserId.as_str(),
    RepositoryGitCloneTokens::CreatedAtUnix.as_str(),
];
const REPOSITORY_GIT_SNAPSHOT_COLUMNS: &[&str] = &[
    RepositoryGitSnapshots::RepoId.as_str(),
    RepositoryGitSnapshots::ObjectKey.as_str(),
    RepositoryGitSnapshots::Sha256.as_str(),
    RepositoryGitSnapshots::GitOid.as_str(),
    RepositoryGitSnapshots::SizeBytes.as_str(),
    RepositoryGitSnapshots::LineCount.as_str(),
];
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
const CLI_DEVICE_LOGIN_COLUMNS: &[&str] = &[
    CliDeviceLogins::DeviceCodeHash.as_str(),
    CliDeviceLogins::UserCodeHash.as_str(),
    CliDeviceLogins::CreatedAtUnix.as_str(),
    CliDeviceLogins::ExpiresAtUnix.as_str(),
    CliDeviceLogins::CompletedUserId.as_str(),
    CliDeviceLogins::CompletedAtUnix.as_str(),
    CliDeviceLogins::ConsumedAtUnix.as_str(),
];
const CLI_BROWSER_LOGIN_COLUMNS: &[&str] = &[
    CliBrowserLogins::RequestId.as_str(),
    CliBrowserLogins::RequestSecretHash.as_str(),
    CliBrowserLogins::CallbackUrl.as_str(),
    CliBrowserLogins::CallbackCodeHash.as_str(),
    CliBrowserLogins::CreatedAtUnix.as_str(),
    CliBrowserLogins::ExpiresAtUnix.as_str(),
    CliBrowserLogins::CompletedUserId.as_str(),
    CliBrowserLogins::CompletedAtUnix.as_str(),
    CliBrowserLogins::ConsumedAtUnix.as_str(),
];
const CLI_EXCHANGE_GRANT_COLUMNS: &[&str] = &[
    CliExchangeGrants::GrantHash.as_str(),
    CliExchangeGrants::UserId.as_str(),
    CliExchangeGrants::CreatedAtUnix.as_str(),
    CliExchangeGrants::ExpiresAtUnix.as_str(),
    CliExchangeGrants::ConsumedAtUnix.as_str(),
];
const CLI_SESSION_COLUMNS: &[&str] = &[
    CliSessions::Id.as_str(),
    CliSessions::TokenHash.as_str(),
    CliSessions::UserId.as_str(),
    CliSessions::Label.as_str(),
    CliSessions::CreatedAtUnix.as_str(),
    CliSessions::LastUsedAtUnix.as_str(),
    CliSessions::ExpiresAtUnix.as_str(),
    CliSessions::RevokedAtUnix.as_str(),
];
const CURRENT_METADATA_DROP_TABLES: &[&str] = &[
    SourceBlobCleanupJobs::Table.as_str(),
    RepoStorageCleanupJobs::Table.as_str(),
    CliSessions::Table.as_str(),
    CliExchangeGrants::Table.as_str(),
    CliBrowserLogins::Table.as_str(),
    CliDeviceLogins::Table.as_str(),
    AuthIdentities::Table.as_str(),
    RepositoryInvites::Table.as_str(),
    RepositoryMembers::Table.as_str(),
    RepositoryGitCloneTokens::Table.as_str(),
    RepositoryGitSnapshots::Table.as_str(),
    RepositoryGitPushTokens::Table.as_str(),
    RepositoryFirstPushTokens::Table.as_str(),
    RepositorySettings::Table.as_str(),
    Repositories::Table.as_str(),
    Users::Table.as_str(),
    MetadataLocks::Table.as_str(),
];
const OBSOLETE_CLI_ACCESS_SESSIONS_TABLE: &str = "scope_cli_access_sessions";
const OBSOLETE_REPO_MEMBERSHIPS_TABLE: &str = "scope_repo_memberships";
const OBSOLETE_METADATA_DROP_TABLES: &[&str] = &[
    OBSOLETE_CLI_ACCESS_SESSIONS_TABLE,
    OBSOLETE_REPO_MEMBERSHIPS_TABLE,
];
pub(super) const METADATA_SCHEMA_TABLES: &[MetadataTableSpec] = &[
    MetadataTableSpec {
        table: MetadataLocks::Table.as_str(),
        columns: METADATA_LOCK_COLUMNS,
        counts_for_catalog_rows: false,
    },
    MetadataTableSpec {
        table: RepoStorageCleanupJobs::Table.as_str(),
        columns: REPO_STORAGE_CLEANUP_JOB_COLUMNS,
        counts_for_catalog_rows: true,
    },
    MetadataTableSpec {
        table: SourceBlobCleanupJobs::Table.as_str(),
        columns: SOURCE_BLOB_CLEANUP_JOB_COLUMNS,
        counts_for_catalog_rows: true,
    },
    MetadataTableSpec {
        table: Users::Table.as_str(),
        columns: USER_COLUMNS,
        counts_for_catalog_rows: true,
    },
    MetadataTableSpec {
        table: AuthIdentities::Table.as_str(),
        columns: AUTH_IDENTITY_COLUMNS,
        counts_for_catalog_rows: true,
    },
    MetadataTableSpec {
        table: Repositories::Table.as_str(),
        columns: REPOSITORY_COLUMNS,
        counts_for_catalog_rows: true,
    },
    MetadataTableSpec {
        table: RepositorySettings::Table.as_str(),
        columns: REPOSITORY_SETTING_COLUMNS,
        counts_for_catalog_rows: true,
    },
    MetadataTableSpec {
        table: RepositoryFirstPushTokens::Table.as_str(),
        columns: REPOSITORY_FIRST_PUSH_TOKEN_COLUMNS,
        counts_for_catalog_rows: true,
    },
    MetadataTableSpec {
        table: RepositoryGitPushTokens::Table.as_str(),
        columns: REPOSITORY_GIT_PUSH_TOKEN_COLUMNS,
        counts_for_catalog_rows: true,
    },
    MetadataTableSpec {
        table: RepositoryGitCloneTokens::Table.as_str(),
        columns: REPOSITORY_GIT_CLONE_TOKEN_COLUMNS,
        counts_for_catalog_rows: true,
    },
    MetadataTableSpec {
        table: RepositoryGitSnapshots::Table.as_str(),
        columns: REPOSITORY_GIT_SNAPSHOT_COLUMNS,
        counts_for_catalog_rows: true,
    },
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
    MetadataTableSpec {
        table: CliDeviceLogins::Table.as_str(),
        columns: CLI_DEVICE_LOGIN_COLUMNS,
        counts_for_catalog_rows: true,
    },
    MetadataTableSpec {
        table: CliBrowserLogins::Table.as_str(),
        columns: CLI_BROWSER_LOGIN_COLUMNS,
        counts_for_catalog_rows: true,
    },
    MetadataTableSpec {
        table: CliExchangeGrants::Table.as_str(),
        columns: CLI_EXCHANGE_GRANT_COLUMNS,
        counts_for_catalog_rows: true,
    },
    MetadataTableSpec {
        table: CliSessions::Table.as_str(),
        columns: CLI_SESSION_COLUMNS,
        counts_for_catalog_rows: true,
    },
];

pub(super) fn metadata_reset_tables() -> Vec<&'static str> {
    CURRENT_METADATA_DROP_TABLES
        .iter()
        .chain(OBSOLETE_METADATA_DROP_TABLES.iter())
        .copied()
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn metadata_schema_inventory_covers_all_reset_tables() {
        let mut reset_tables = metadata_reset_tables();
        reset_tables.sort_unstable();
        reset_tables.dedup();

        for table in METADATA_SCHEMA_TABLES {
            assert!(
                reset_tables.contains(&table.table),
                "reset list missing {}",
                table.table
            );
        }
        assert!(reset_tables.contains(&OBSOLETE_CLI_ACCESS_SESSIONS_TABLE));
        assert!(reset_tables.contains(&OBSOLETE_REPO_MEMBERSHIPS_TABLE));
    }

    #[test]
    fn metadata_schema_inventory_marks_catalog_row_tables() {
        let catalog_tables = METADATA_SCHEMA_TABLES
            .iter()
            .filter(|table| table.counts_for_catalog_rows)
            .map(|table| table.table)
            .collect::<Vec<_>>();

        assert_eq!(
            catalog_tables,
            vec![
                RepoStorageCleanupJobs::Table.as_str(),
                SourceBlobCleanupJobs::Table.as_str(),
                Users::Table.as_str(),
                AuthIdentities::Table.as_str(),
                Repositories::Table.as_str(),
                RepositorySettings::Table.as_str(),
                RepositoryFirstPushTokens::Table.as_str(),
                RepositoryGitPushTokens::Table.as_str(),
                RepositoryGitCloneTokens::Table.as_str(),
                RepositoryGitSnapshots::Table.as_str(),
                RepositoryMembers::Table.as_str(),
                RepositoryInvites::Table.as_str(),
                CliDeviceLogins::Table.as_str(),
                CliBrowserLogins::Table.as_str(),
                CliExchangeGrants::Table.as_str(),
                CliSessions::Table.as_str(),
            ]
        );
    }

    #[test]
    fn metadata_schema_inventory_has_unique_names() {
        let mut tables = std::collections::BTreeSet::new();
        for table in METADATA_SCHEMA_TABLES {
            assert!(
                tables.insert(table.table),
                "duplicate table {}",
                table.table
            );
            assert!(!table.columns.is_empty(), "{} has no columns", table.table);

            let mut columns = std::collections::BTreeSet::new();
            for column in table.columns {
                assert!(
                    columns.insert(*column),
                    "duplicate column {}.{}",
                    table.table,
                    column
                );
            }
        }
    }
}
