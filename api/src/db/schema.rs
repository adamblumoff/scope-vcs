use sea_orm::{ConnectionTrait, Statement};
use sea_orm::{DatabaseConnection, DbErr};
use sea_orm_migration::{manager::SchemaManager, prelude::*};

pub(crate) async fn migrate_metadata_schema(db: &DatabaseConnection) -> Result<(), DbErr> {
    let manager = SchemaManager::new(db);
    ensure_metadata_reset_events_table(&manager).await?;
    if let Some(drift) = metadata_schema_drift(&manager).await? {
        if !metadata_schema_has_catalog_rows(db, &manager).await?
            || is_destructive_pre_alpha_reset_drift(&drift)
        {
            reset_metadata_schema(db).await?;
            ensure_metadata_reset_events_table(&manager).await?;
        } else {
            return Err(DbErr::Custom(format!(
                "Scope metadata schema drift detected: {drift}; reset the metadata schema explicitly before starting this pre-alpha server"
            )));
        }
    }

    manager
        .create_table(
            Table::create()
                .table(MetadataLocks::Table)
                .if_not_exists()
                .col(
                    ColumnDef::new(MetadataLocks::Key)
                        .string()
                        .not_null()
                        .primary_key(),
                )
                .col(
                    ColumnDef::new(MetadataLocks::PendingRepoStorageDeletions)
                        .json_binary()
                        .not_null(),
                )
                .col(
                    ColumnDef::new(MetadataLocks::PendingSourceBlobDeletions)
                        .json_binary()
                        .not_null(),
                )
                .to_owned(),
        )
        .await?;

    manager
        .create_table(
            Table::create()
                .table(Users::Table)
                .if_not_exists()
                .col(ColumnDef::new(Users::Id).string().not_null().primary_key())
                .col(
                    ColumnDef::new(Users::Handle)
                        .string()
                        .not_null()
                        .unique_key(),
                )
                .col(ColumnDef::new(Users::Email).string().not_null())
                .col(ColumnDef::new(Users::EmailVerified).boolean().not_null())
                .col(ColumnDef::new(Users::Access).string().not_null())
                .to_owned(),
        )
        .await?;

    manager
        .create_table(
            Table::create()
                .table(AuthIdentities::Table)
                .if_not_exists()
                .col(ColumnDef::new(AuthIdentities::Provider).string().not_null())
                .col(ColumnDef::new(AuthIdentities::Subject).string().not_null())
                .col(ColumnDef::new(AuthIdentities::UserId).string().not_null())
                .primary_key(
                    Index::create()
                        .name("pk_scope_auth_identities")
                        .col(AuthIdentities::Provider)
                        .col(AuthIdentities::Subject),
                )
                .foreign_key(
                    ForeignKey::create()
                        .name("fk_scope_auth_identities_user")
                        .from(AuthIdentities::Table, AuthIdentities::UserId)
                        .to(Users::Table, Users::Id)
                        .on_delete(ForeignKeyAction::Cascade),
                )
                .to_owned(),
        )
        .await?;

    manager
        .create_index(
            Index::create()
                .name("idx_scope_auth_identities_user")
                .table(AuthIdentities::Table)
                .col(AuthIdentities::UserId)
                .if_not_exists()
                .to_owned(),
        )
        .await?;

    manager
        .create_table(
            Table::create()
                .table(Repositories::Table)
                .if_not_exists()
                .col(
                    ColumnDef::new(Repositories::Id)
                        .string()
                        .not_null()
                        .primary_key(),
                )
                .col(
                    ColumnDef::new(Repositories::OwnerHandle)
                        .string()
                        .not_null(),
                )
                .col(ColumnDef::new(Repositories::Name).string().not_null())
                .col(
                    ColumnDef::new(Repositories::OwnerUserId)
                        .string()
                        .not_null(),
                )
                .col(
                    ColumnDef::new(Repositories::PublicationState)
                        .string()
                        .not_null(),
                )
                .col(
                    ColumnDef::new(Repositories::DefaultVisibility)
                        .string()
                        .not_null(),
                )
                .col(
                    ColumnDef::new(Repositories::Settings)
                        .json_binary()
                        .not_null(),
                )
                .col(ColumnDef::new(Repositories::FirstPushToken).json_binary())
                .col(ColumnDef::new(Repositories::GitPushToken).json_binary())
                .col(
                    ColumnDef::new(Repositories::GitCloneTokens)
                        .json_binary()
                        .not_null(),
                )
                .col(ColumnDef::new(Repositories::PendingImport).json_binary())
                .col(
                    ColumnDef::new(Repositories::Policy)
                        .json_binary()
                        .not_null(),
                )
                .col(ColumnDef::new(Repositories::Graph).json_binary().not_null())
                .col(ColumnDef::new(Repositories::GitSnapshot).json_binary())
                .col(ColumnDef::new(Repositories::StagedUpdate).json_binary())
                .col(
                    ColumnDef::new(Repositories::Invitations)
                        .json_binary()
                        .not_null(),
                )
                .foreign_key(
                    ForeignKey::create()
                        .name("fk_scope_repositories_owner")
                        .from(Repositories::Table, Repositories::OwnerUserId)
                        .to(Users::Table, Users::Id)
                        .on_delete(ForeignKeyAction::Cascade),
                )
                .to_owned(),
        )
        .await?;

    manager
        .create_index(
            Index::create()
                .name("idx_scope_repositories_owner_name")
                .table(Repositories::Table)
                .col(Repositories::OwnerHandle)
                .col(Repositories::Name)
                .unique()
                .if_not_exists()
                .to_owned(),
        )
        .await?;

    manager
        .create_table(
            Table::create()
                .table(Memberships::Table)
                .if_not_exists()
                .col(ColumnDef::new(Memberships::RepoId).string().not_null())
                .col(ColumnDef::new(Memberships::UserId).string().not_null())
                .col(ColumnDef::new(Memberships::Role).string().not_null())
                .primary_key(
                    Index::create()
                        .name("pk_scope_repo_memberships")
                        .col(Memberships::RepoId)
                        .col(Memberships::UserId),
                )
                .foreign_key(
                    ForeignKey::create()
                        .name("fk_scope_repo_memberships_repo")
                        .from(Memberships::Table, Memberships::RepoId)
                        .to(Repositories::Table, Repositories::Id)
                        .on_delete(ForeignKeyAction::Cascade),
                )
                .foreign_key(
                    ForeignKey::create()
                        .name("fk_scope_repo_memberships_user")
                        .from(Memberships::Table, Memberships::UserId)
                        .to(Users::Table, Users::Id)
                        .on_delete(ForeignKeyAction::Cascade),
                )
                .to_owned(),
        )
        .await?;

    manager
        .create_index(
            Index::create()
                .name("idx_scope_repo_memberships_user")
                .table(Memberships::Table)
                .col(Memberships::UserId)
                .if_not_exists()
                .to_owned(),
        )
        .await?;

    manager
        .create_table(
            Table::create()
                .table(CliDeviceLogins::Table)
                .if_not_exists()
                .col(
                    ColumnDef::new(CliDeviceLogins::DeviceCodeHash)
                        .string()
                        .not_null()
                        .primary_key(),
                )
                .col(
                    ColumnDef::new(CliDeviceLogins::UserCodeHash)
                        .string()
                        .not_null()
                        .unique_key(),
                )
                .col(
                    ColumnDef::new(CliDeviceLogins::CreatedAtUnix)
                        .big_integer()
                        .not_null(),
                )
                .col(
                    ColumnDef::new(CliDeviceLogins::ExpiresAtUnix)
                        .big_integer()
                        .not_null(),
                )
                .col(ColumnDef::new(CliDeviceLogins::CompletedUserId).string())
                .col(ColumnDef::new(CliDeviceLogins::CompletedAtUnix).big_integer())
                .col(ColumnDef::new(CliDeviceLogins::ConsumedAtUnix).big_integer())
                .foreign_key(
                    ForeignKey::create()
                        .name("fk_scope_cli_device_logins_completed_user")
                        .from(CliDeviceLogins::Table, CliDeviceLogins::CompletedUserId)
                        .to(Users::Table, Users::Id)
                        .on_delete(ForeignKeyAction::Cascade),
                )
                .to_owned(),
        )
        .await?;

    manager
        .create_table(
            Table::create()
                .table(CliBrowserLogins::Table)
                .if_not_exists()
                .col(
                    ColumnDef::new(CliBrowserLogins::RequestId)
                        .string()
                        .not_null()
                        .primary_key(),
                )
                .col(
                    ColumnDef::new(CliBrowserLogins::RequestSecretHash)
                        .string()
                        .not_null(),
                )
                .col(
                    ColumnDef::new(CliBrowserLogins::CallbackUrl)
                        .text()
                        .not_null(),
                )
                .col(ColumnDef::new(CliBrowserLogins::CallbackCodeHash).string())
                .col(
                    ColumnDef::new(CliBrowserLogins::CreatedAtUnix)
                        .big_integer()
                        .not_null(),
                )
                .col(
                    ColumnDef::new(CliBrowserLogins::ExpiresAtUnix)
                        .big_integer()
                        .not_null(),
                )
                .col(ColumnDef::new(CliBrowserLogins::CompletedUserId).string())
                .col(ColumnDef::new(CliBrowserLogins::CompletedAtUnix).big_integer())
                .col(ColumnDef::new(CliBrowserLogins::ConsumedAtUnix).big_integer())
                .foreign_key(
                    ForeignKey::create()
                        .name("fk_scope_cli_browser_logins_completed_user")
                        .from(CliBrowserLogins::Table, CliBrowserLogins::CompletedUserId)
                        .to(Users::Table, Users::Id)
                        .on_delete(ForeignKeyAction::Cascade),
                )
                .to_owned(),
        )
        .await?;

    manager
        .create_table(
            Table::create()
                .table(CliExchangeGrants::Table)
                .if_not_exists()
                .col(
                    ColumnDef::new(CliExchangeGrants::GrantHash)
                        .string()
                        .not_null()
                        .primary_key(),
                )
                .col(
                    ColumnDef::new(CliExchangeGrants::UserId)
                        .string()
                        .not_null(),
                )
                .col(
                    ColumnDef::new(CliExchangeGrants::CreatedAtUnix)
                        .big_integer()
                        .not_null(),
                )
                .col(
                    ColumnDef::new(CliExchangeGrants::ExpiresAtUnix)
                        .big_integer()
                        .not_null(),
                )
                .col(ColumnDef::new(CliExchangeGrants::ConsumedAtUnix).big_integer())
                .foreign_key(
                    ForeignKey::create()
                        .name("fk_scope_cli_exchange_grants_user")
                        .from(CliExchangeGrants::Table, CliExchangeGrants::UserId)
                        .to(Users::Table, Users::Id)
                        .on_delete(ForeignKeyAction::Cascade),
                )
                .to_owned(),
        )
        .await?;

    manager
        .create_index(
            Index::create()
                .name("idx_scope_cli_exchange_grants_user")
                .table(CliExchangeGrants::Table)
                .col(CliExchangeGrants::UserId)
                .if_not_exists()
                .to_owned(),
        )
        .await?;

    manager
        .create_table(
            Table::create()
                .table(CliSessions::Table)
                .if_not_exists()
                .col(
                    ColumnDef::new(CliSessions::Id)
                        .string()
                        .not_null()
                        .primary_key(),
                )
                .col(
                    ColumnDef::new(CliSessions::TokenHash)
                        .string()
                        .not_null()
                        .unique_key(),
                )
                .col(ColumnDef::new(CliSessions::UserId).string().not_null())
                .col(ColumnDef::new(CliSessions::Label).string().not_null())
                .col(
                    ColumnDef::new(CliSessions::CreatedAtUnix)
                        .big_integer()
                        .not_null(),
                )
                .col(ColumnDef::new(CliSessions::LastUsedAtUnix).big_integer())
                .col(
                    ColumnDef::new(CliSessions::ExpiresAtUnix)
                        .big_integer()
                        .not_null(),
                )
                .col(ColumnDef::new(CliSessions::RevokedAtUnix).big_integer())
                .foreign_key(
                    ForeignKey::create()
                        .name("fk_scope_cli_sessions_user")
                        .from(CliSessions::Table, CliSessions::UserId)
                        .to(Users::Table, Users::Id)
                        .on_delete(ForeignKeyAction::Cascade),
                )
                .to_owned(),
        )
        .await?;

    manager
        .create_index(
            Index::create()
                .name("idx_scope_cli_sessions_user")
                .table(CliSessions::Table)
                .col(CliSessions::UserId)
                .if_not_exists()
                .to_owned(),
        )
        .await?;

    Ok(())
}

async fn ensure_metadata_reset_events_table(manager: &SchemaManager<'_>) -> Result<(), DbErr> {
    manager
        .create_table(
            Table::create()
                .table(MetadataResetEvents::Table)
                .if_not_exists()
                .col(
                    ColumnDef::new(MetadataResetEvents::Id)
                        .string()
                        .not_null()
                        .primary_key(),
                )
                .col(
                    ColumnDef::new(MetadataResetEvents::ResetAtUnix)
                        .big_integer()
                        .not_null(),
                )
                .col(
                    ColumnDef::new(MetadataResetEvents::Trigger)
                        .string()
                        .not_null(),
                )
                .col(
                    ColumnDef::new(MetadataResetEvents::Reason)
                        .text()
                        .not_null(),
                )
                .to_owned(),
        )
        .await
}

pub(crate) async fn reset_metadata_schema(db: &DatabaseConnection) -> Result<(), DbErr> {
    let backend = db.get_database_backend();
    db.execute(Statement::from_string(
        backend,
        [
            "DROP TABLE IF EXISTS",
            "scope_cli_sessions,",
            "scope_cli_exchange_grants,",
            "scope_cli_browser_logins,",
            "scope_cli_device_logins,",
            "scope_cli_access_sessions,",
            "scope_auth_identities,",
            "scope_repo_memberships,",
            "scope_repositories,",
            "scope_users,",
            "scope_metadata_locks",
            "CASCADE",
        ]
        .join(" "),
    ))
    .await?;
    Ok(())
}

async fn metadata_schema_has_catalog_rows(
    db: &DatabaseConnection,
    manager: &SchemaManager<'_>,
) -> Result<bool, DbErr> {
    let backend = db.get_database_backend();
    for table in [
        "scope_users",
        "scope_auth_identities",
        "scope_repositories",
        "scope_repo_memberships",
        "scope_cli_device_logins",
        "scope_cli_browser_logins",
        "scope_cli_exchange_grants",
        "scope_cli_sessions",
    ] {
        if !manager.has_table(table).await? {
            continue;
        }
        let row = db
            .query_one(Statement::from_string(
                backend,
                format!("SELECT 1 FROM {table} LIMIT 1"),
            ))
            .await?;
        if row.is_some() {
            return Ok(true);
        }
    }

    if manager.has_table("scope_metadata_locks").await? {
        for column in [
            "pending_repo_storage_deletions",
            "pending_source_blob_deletions",
        ] {
            if !manager.has_column("scope_metadata_locks", column).await? {
                continue;
            }
            let row = db
                .query_one(Statement::from_string(
                    backend,
                    format!(
                        "SELECT 1 FROM scope_metadata_locks WHERE jsonb_array_length({column}) > 0 LIMIT 1"
                    ),
                ))
                .await?;
            if row.is_some() {
                return Ok(true);
            }
        }
    }
    Ok(false)
}

async fn metadata_schema_drift(manager: &SchemaManager<'_>) -> Result<Option<String>, DbErr> {
    let tables = [
        (
            "scope_metadata_locks",
            &[
                "key",
                "pending_repo_storage_deletions",
                "pending_source_blob_deletions",
            ][..],
        ),
        (
            "scope_users",
            &["id", "handle", "email", "email_verified", "access"][..],
        ),
        (
            "scope_auth_identities",
            &["provider", "subject", "user_id"][..],
        ),
        (
            "scope_repositories",
            &[
                "id",
                "owner_handle",
                "name",
                "owner_user_id",
                "publication_state",
                "default_visibility",
                "settings",
                "first_push_token",
                "git_push_token",
                "git_clone_tokens",
                "pending_import",
                "policy",
                "graph",
                "git_snapshot",
                "staged_update",
                "invitations",
            ][..],
        ),
        (
            "scope_repo_memberships",
            &["repo_id", "user_id", "role"][..],
        ),
        (
            "scope_cli_device_logins",
            &[
                "device_code_hash",
                "user_code_hash",
                "created_at_unix",
                "expires_at_unix",
                "completed_user_id",
                "completed_at_unix",
                "consumed_at_unix",
            ][..],
        ),
        (
            "scope_cli_browser_logins",
            &[
                "request_id",
                "request_secret_hash",
                "callback_url",
                "callback_code_hash",
                "created_at_unix",
                "expires_at_unix",
                "completed_user_id",
                "completed_at_unix",
                "consumed_at_unix",
            ][..],
        ),
        (
            "scope_cli_exchange_grants",
            &[
                "grant_hash",
                "user_id",
                "created_at_unix",
                "expires_at_unix",
                "consumed_at_unix",
            ][..],
        ),
        (
            "scope_cli_sessions",
            &[
                "id",
                "token_hash",
                "user_id",
                "label",
                "created_at_unix",
                "last_used_at_unix",
                "expires_at_unix",
                "revoked_at_unix",
            ][..],
        ),
    ];

    for (table, columns) in tables {
        if !manager.has_table(table).await? {
            return Ok(Some(format!("missing table {table}")));
        }
        for column in columns {
            if !manager.has_column(table, column).await? {
                return Ok(Some(format!("missing column {table}.{column}")));
            }
        }
    }
    Ok(None)
}

fn is_destructive_pre_alpha_reset_drift(drift: &str) -> bool {
    drift.starts_with("missing table ") || drift.starts_with("missing column ")
}

macro_rules! impl_iden {
    ($name:ident { $($variant:ident => $value:literal),+ $(,)? }) => {
        impl Iden for $name {
            fn unquoted(&self, s: &mut dyn std::fmt::Write) {
                let value = match self {
                    $(Self::$variant => $value,)+
                };
                std::fmt::Write::write_str(s, value).expect("writing identifier cannot fail");
            }
        }
    };
}

#[derive(Copy, Clone)]
enum Users {
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
enum AuthIdentities {
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
enum Repositories {
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
enum CliDeviceLogins {
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
enum CliBrowserLogins {
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
enum CliExchangeGrants {
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
enum CliSessions {
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
enum Memberships {
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
enum MetadataLocks {
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
enum MetadataResetEvents {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn destructive_pre_alpha_reset_drift_allows_pre_alpha_shape_changes() {
        assert!(is_destructive_pre_alpha_reset_drift(
            "missing column scope_repositories.git_clone_tokens"
        ));
        assert!(is_destructive_pre_alpha_reset_drift(
            "missing column scope_repositories.owner_user_id"
        ));
        assert!(is_destructive_pre_alpha_reset_drift(
            "missing column scope_users.email"
        ));
        assert!(is_destructive_pre_alpha_reset_drift(
            "missing table scope_auth_identities"
        ));
    }
}
