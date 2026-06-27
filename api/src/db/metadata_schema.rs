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
    Settings,
    FirstPushToken,
    GitPushToken,
    GitCloneTokens,
    PendingImport,
    Policy,
    Graph,
    GitSnapshot,
    StagedUpdate,
    Invitations,
}

impl_iden!(Repositories {
    Table => "scope_repositories",
    Id => "id",
    OwnerHandle => "owner_handle",
    Name => "name",
    OwnerUserId => "owner_user_id",
    PublicationState => "publication_state",
    DefaultVisibility => "default_visibility",
    Settings => "settings",
    FirstPushToken => "first_push_token",
    GitPushToken => "git_push_token",
    GitCloneTokens => "git_clone_tokens",
    PendingImport => "pending_import",
    Policy => "policy",
    Graph => "graph",
    GitSnapshot => "git_snapshot",
    StagedUpdate => "staged_update",
    Invitations => "invitations",
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
pub(super) enum Memberships {
    Table,
    RepoId,
    UserId,
    Role,
}

impl_iden!(Memberships {
    Table => "scope_repo_memberships",
    RepoId => "repo_id",
    UserId => "user_id",
    Role => "role",
});

#[derive(Copy, Clone)]
pub(super) enum MetadataLocks {
    Table,
    Key,
    PendingRepoStorageDeletions,
    PendingSourceBlobDeletions,
}

impl_iden!(MetadataLocks {
    Table => "scope_metadata_locks",
    Key => "key",
    PendingRepoStorageDeletions => "pending_repo_storage_deletions",
    PendingSourceBlobDeletions => "pending_source_blob_deletions",
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

const METADATA_LOCK_COLUMNS: &[&str] = &[
    MetadataLocks::Key.as_str(),
    MetadataLocks::PendingRepoStorageDeletions.as_str(),
    MetadataLocks::PendingSourceBlobDeletions.as_str(),
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
    Repositories::Settings.as_str(),
    Repositories::FirstPushToken.as_str(),
    Repositories::GitPushToken.as_str(),
    Repositories::GitCloneTokens.as_str(),
    Repositories::PendingImport.as_str(),
    Repositories::Policy.as_str(),
    Repositories::Graph.as_str(),
    Repositories::GitSnapshot.as_str(),
    Repositories::StagedUpdate.as_str(),
    Repositories::Invitations.as_str(),
];
const MEMBERSHIP_COLUMNS: &[&str] = &[
    Memberships::RepoId.as_str(),
    Memberships::UserId.as_str(),
    Memberships::Role.as_str(),
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
pub(super) const METADATA_LOCK_CATALOG_COLUMNS: &[&str] = &[
    MetadataLocks::PendingRepoStorageDeletions.as_str(),
    MetadataLocks::PendingSourceBlobDeletions.as_str(),
];
const CURRENT_METADATA_DROP_TABLES: &[&str] = &[
    CliSessions::Table.as_str(),
    CliExchangeGrants::Table.as_str(),
    CliBrowserLogins::Table.as_str(),
    CliDeviceLogins::Table.as_str(),
    AuthIdentities::Table.as_str(),
    Memberships::Table.as_str(),
    Repositories::Table.as_str(),
    Users::Table.as_str(),
    MetadataLocks::Table.as_str(),
];
const OBSOLETE_CLI_ACCESS_SESSIONS_TABLE: &str = "scope_cli_access_sessions";
const OBSOLETE_METADATA_DROP_TABLES: &[&str] = &[OBSOLETE_CLI_ACCESS_SESSIONS_TABLE];
pub(super) const METADATA_SCHEMA_TABLES: &[MetadataTableSpec] = &[
    MetadataTableSpec {
        table: MetadataLocks::Table.as_str(),
        columns: METADATA_LOCK_COLUMNS,
        counts_for_catalog_rows: false,
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
        table: Memberships::Table.as_str(),
        columns: MEMBERSHIP_COLUMNS,
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
                Users::Table.as_str(),
                AuthIdentities::Table.as_str(),
                Repositories::Table.as_str(),
                Memberships::Table.as_str(),
                CliDeviceLogins::Table.as_str(),
                CliBrowserLogins::Table.as_str(),
                CliExchangeGrants::Table.as_str(),
                CliSessions::Table.as_str(),
            ]
        );
        assert_eq!(
            METADATA_LOCK_CATALOG_COLUMNS,
            &[
                MetadataLocks::PendingRepoStorageDeletions.as_str(),
                MetadataLocks::PendingSourceBlobDeletions.as_str(),
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
