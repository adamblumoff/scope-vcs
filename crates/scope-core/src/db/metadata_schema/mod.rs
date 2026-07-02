use sea_orm_migration::prelude::Iden;

#[derive(Copy, Clone)]
pub struct MetadataTableSpec {
    pub table: &'static str,
    pub columns: &'static [&'static str],
    pub counts_for_catalog_rows: bool,
}

macro_rules! impl_iden {
    ($name:ident { $($variant:ident => $value:literal),+ $(,)? }) => {
        impl $name {
            pub const fn as_str(self) -> &'static str {
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

mod auth;
mod collaboration;
mod jobs;
mod read_models;
mod repositories;

pub use auth::{
    AuthIdentities, CliBrowserLogins, CliDeviceLogins, CliExchangeGrants, CliSessions, Users,
};
pub use collaboration::{RepositoryInvites, RepositoryMembers};
pub use jobs::{
    MetadataLocks, MetadataResetEvents, OutboxJobs, RepoStorageCleanupJobs, SourceBlobCleanupJobs,
};
pub use read_models::{ProjectionFiles, ProjectionReadModels};
pub use repositories::{
    Repositories, RepositoryFirstPushTokens, RepositoryGitCloneTokens, RepositoryGitPushTokens,
    RepositoryGitSnapshots, RepositorySettings,
};

const METADATA_SCHEMA_TABLE_GROUPS: &[&[MetadataTableSpec]] = &[
    jobs::LOCK_AND_CLEANUP_TABLES,
    auth::USER_IDENTITY_TABLES,
    repositories::TABLES,
    read_models::TABLES,
    jobs::OUTBOX_TABLES,
    collaboration::TABLES,
    auth::CLI_TABLES,
];

const CURRENT_METADATA_DROP_TABLES: &[&str] = &[
    OutboxJobs::Table.as_str(),
    SourceBlobCleanupJobs::Table.as_str(),
    RepoStorageCleanupJobs::Table.as_str(),
    CliSessions::Table.as_str(),
    CliExchangeGrants::Table.as_str(),
    CliBrowserLogins::Table.as_str(),
    CliDeviceLogins::Table.as_str(),
    AuthIdentities::Table.as_str(),
    RepositoryInvites::Table.as_str(),
    RepositoryMembers::Table.as_str(),
    ProjectionFiles::Table.as_str(),
    ProjectionReadModels::Table.as_str(),
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

pub fn metadata_schema_tables() -> impl Iterator<Item = &'static MetadataTableSpec> {
    METADATA_SCHEMA_TABLE_GROUPS
        .iter()
        .flat_map(|group| group.iter())
}

pub fn metadata_reset_tables() -> Vec<&'static str> {
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

        for table in metadata_schema_tables() {
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
        let catalog_tables = metadata_schema_tables()
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
        for table in metadata_schema_tables() {
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
