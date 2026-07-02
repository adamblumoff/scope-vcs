use super::metadata_schema::*;
use super::schema_auth::ensure_auth_tables;
use super::schema_cleanup::ensure_metadata_lock_and_cleanup_tables;
use super::schema_collaboration::ensure_repository_collaboration_tables;
use super::schema_outbox::ensure_outbox_tables;
use super::schema_read_models::ensure_read_model_tables;
use super::schema_repositories::ensure_repository_tables;
use super::schema_repository_facts::ensure_repository_fact_tables;
use sea_orm::{ConnectionTrait, Statement};
use sea_orm::{DatabaseConnection, DbErr};
use sea_orm_migration::{manager::SchemaManager, prelude::*};

pub async fn migrate_metadata_schema(db: &DatabaseConnection) -> Result<(), DbErr> {
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

    ensure_metadata_lock_and_cleanup_tables(&manager).await?;
    ensure_auth_tables(&manager).await?;
    ensure_repository_tables(&manager).await?;
    ensure_repository_fact_tables(&manager).await?;
    ensure_read_model_tables(&manager).await?;
    ensure_outbox_tables(&manager).await?;
    ensure_repository_collaboration_tables(&manager).await?;

    Ok(())
}

pub async fn assert_metadata_schema_ready(db: &DatabaseConnection) -> Result<(), DbErr> {
    let manager = SchemaManager::new(db);
    if let Some(drift) = metadata_schema_drift(&manager).await? {
        return Err(DbErr::Custom(format!(
            "Scope metadata schema is not ready for workers: {drift}"
        )));
    }

    let lock_row = db
        .query_one(Statement::from_string(
            db.get_database_backend(),
            format!("SELECT 1 FROM {} LIMIT 1", MetadataLocks::Table.as_str()),
        ))
        .await?;
    if lock_row.is_none() {
        return Err(DbErr::Custom(
            "Scope metadata lock row is not ready for workers".to_string(),
        ));
    }

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

pub async fn reset_metadata_schema(db: &DatabaseConnection) -> Result<(), DbErr> {
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
    for table in metadata_schema_tables()
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

    Ok(false)
}

async fn metadata_schema_drift(manager: &SchemaManager<'_>) -> Result<Option<String>, DbErr> {
    for table in metadata_schema_tables() {
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

    #[tokio::test]
    async fn creates_users_table_before_user_email_index() {
        use sea_orm::{DbBackend, MockDatabase, MockExecResult};

        let db = MockDatabase::new(DbBackend::Postgres)
            .append_exec_results(vec![MockExecResult::default(); 10])
            .into_connection();
        let manager = SchemaManager::new(&db);

        ensure_auth_tables(&manager).await.unwrap();

        let sql = db
            .into_transaction_log()
            .into_iter()
            .flat_map(|transaction| {
                transaction
                    .statements()
                    .iter()
                    .map(|statement| statement.sql.clone())
                    .collect::<Vec<_>>()
            })
            .collect::<Vec<_>>();
        let users_table = sql
            .iter()
            .position(|statement| statement.contains("CREATE TABLE IF NOT EXISTS \"scope_users\""))
            .expect("scope_users table should be created");
        let email_index = sql
            .iter()
            .position(|statement| statement.contains("idx_scope_users_email"))
            .expect("scope_users email index should be created");

        assert!(
            users_table < email_index,
            "scope_users must be created before idx_scope_users_email"
        );
    }

    #[test]
    fn destructive_pre_alpha_reset_drift_allows_pre_alpha_shape_changes() {
        assert!(is_destructive_pre_alpha_reset_drift(
            "missing table scope_repository_git_clone_tokens"
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
