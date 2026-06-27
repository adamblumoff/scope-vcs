use super::metadata_schema::*;
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
    if metadata_schema_has_duplicate_user_emails(db, &manager).await? {
        reset_metadata_schema(db).await?;
        ensure_metadata_reset_events_table(&manager).await?;
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
        .create_index(
            Index::create()
                .name("idx_scope_users_email")
                .table(Users::Table)
                .col(Users::Email)
                .unique()
                .if_not_exists()
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
    let tables = metadata_reset_tables().join(", ");
    db.execute(Statement::from_string(
        backend,
        format!("DROP TABLE IF EXISTS {tables} CASCADE"),
    ))
    .await?;
    Ok(())
}

async fn metadata_schema_has_catalog_rows(
    db: &DatabaseConnection,
    manager: &SchemaManager<'_>,
) -> Result<bool, DbErr> {
    let backend = db.get_database_backend();
    for table in METADATA_SCHEMA_TABLES
        .iter()
        .filter(|table| table.counts_for_catalog_rows)
        .map(|table| table.table)
    {
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

    let metadata_locks_table = MetadataLocks::Table.as_str();
    if manager.has_table(metadata_locks_table).await? {
        for column in METADATA_LOCK_CATALOG_COLUMNS.iter().copied() {
            if !manager.has_column(metadata_locks_table, column).await? {
                continue;
            }
            let row = db
                .query_one(Statement::from_string(
                    backend,
                    format!(
                        "SELECT 1 FROM {metadata_locks_table} WHERE jsonb_array_length({column}) > 0 LIMIT 1"
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
    for table in METADATA_SCHEMA_TABLES {
        if !manager.has_table(table.table).await? {
            return Ok(Some(format!("missing table {}", table.table)));
        }
        for column in table.columns.iter().copied() {
            if !manager.has_column(table.table, column).await? {
                return Ok(Some(format!("missing column {}.{column}", table.table)));
            }
        }
    }
    Ok(None)
}

async fn metadata_schema_has_duplicate_user_emails(
    db: &DatabaseConnection,
    manager: &SchemaManager<'_>,
) -> Result<bool, DbErr> {
    if !manager.has_table(Users::Table.as_str()).await?
        || !manager
            .has_column(Users::Table.as_str(), Users::Email.as_str())
            .await?
    {
        return Ok(false);
    }

    let row = db
        .query_one(Statement::from_string(
            db.get_database_backend(),
            format!(
                "SELECT {email} FROM {users} WHERE {email} <> '' GROUP BY {email} HAVING COUNT(*) > 1 LIMIT 1",
                users = Users::Table.as_str(),
                email = Users::Email.as_str(),
            ),
        ))
        .await?;
    Ok(row.is_some())
}

fn is_destructive_pre_alpha_reset_drift(drift: &str) -> bool {
    drift.starts_with("missing table ") || drift.starts_with("missing column ")
}

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
