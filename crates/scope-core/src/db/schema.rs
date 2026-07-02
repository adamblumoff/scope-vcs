use super::schema_contract::{
    self, ColumnSpec, ColumnType, ForeignKeyActionSpec, ForeignKeySpec, IndexSpec, PrimaryKeySpec,
    SchemaIden, TableSpec,
};
use sea_orm::{ConnectionTrait, QueryResult, Statement};
use sea_orm::{DatabaseConnection, DbErr};
use sea_orm_migration::{
    manager::SchemaManager,
    prelude::*,
    sea_query::{ForeignKeyCreateStatement, IndexCreateStatement, TableCreateStatement},
};

pub async fn migrate_metadata_schema(db: &DatabaseConnection) -> Result<(), DbErr> {
    let manager = SchemaManager::new(db);
    ensure_metadata_reset_events_table(&manager).await?;
    if let Some(drift) = metadata_schema_drift(db, &manager).await? {
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

    ensure_schema_tables(&manager).await?;

    Ok(())
}

pub async fn assert_metadata_schema_ready(db: &DatabaseConnection) -> Result<(), DbErr> {
    let manager = SchemaManager::new(db);
    if let Some(drift) = metadata_schema_drift(db, &manager).await? {
        return Err(DbErr::Custom(format!(
            "Scope metadata schema is not ready for workers: {drift}"
        )));
    }

    let lock_row = db
        .query_one(Statement::from_string(
            db.get_database_backend(),
            format!(
                "SELECT 1 FROM {} LIMIT 1",
                schema_contract::jobs::METADATA_LOCKS.name
            ),
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
    ensure_table(manager, schema_contract::metadata_reset_events_table()).await
}

async fn ensure_schema_tables(manager: &SchemaManager<'_>) -> Result<(), DbErr> {
    for table in schema_contract::schema_tables() {
        ensure_table(manager, table).await?;
    }

    for table in schema_contract::schema_tables() {
        for index in table.indexes {
            manager
                .create_index(create_index_statement(table, index))
                .await?;
        }
    }

    Ok(())
}

async fn ensure_table(manager: &SchemaManager<'_>, table: &TableSpec) -> Result<(), DbErr> {
    manager.create_table(create_table_statement(table)).await
}

fn create_table_statement(table: &TableSpec) -> TableCreateStatement {
    let mut statement = Table::create();
    statement.table(SchemaIden::new(table.name)).if_not_exists();

    let inline_primary_key = table.inline_primary_key_column();
    for column in table.columns {
        statement.col(column_definition(
            column,
            inline_primary_key == Some(column.name),
        ));
    }

    if let PrimaryKeySpec::Composite { name, columns } = table.primary_key {
        let mut primary_key = Index::create();
        primary_key.name(name);
        for column in columns.iter().copied() {
            primary_key.col(SchemaIden::new(column));
        }
        statement.primary_key(&mut primary_key);
    }

    for foreign_key in table.foreign_keys {
        let mut foreign_key = foreign_key_statement(table.name, foreign_key);
        statement.foreign_key(&mut foreign_key);
    }

    statement
}

fn column_definition(column: &ColumnSpec, primary_key: bool) -> ColumnDef {
    let mut definition = ColumnDef::new(SchemaIden::new(column.name));
    match column.column_type {
        ColumnType::BigInteger => {
            definition.big_integer();
        }
        ColumnType::Boolean => {
            definition.boolean();
        }
        ColumnType::Integer => {
            definition.integer();
        }
        ColumnType::JsonBinary => {
            definition.json_binary();
        }
        ColumnType::String => {
            definition.string();
        }
        ColumnType::Text => {
            definition.text();
        }
    }

    if !column.nullable {
        definition.not_null();
    }
    if primary_key {
        definition.primary_key();
    }
    if column.unique {
        definition.unique_key();
    }

    definition
}

fn create_index_statement(table: &TableSpec, index: &IndexSpec) -> IndexCreateStatement {
    let mut statement = Index::create();
    statement
        .name(index.name)
        .table(SchemaIden::new(table.name))
        .if_not_exists();
    for column in index.columns.iter().copied() {
        statement.col(SchemaIden::new(column));
    }
    if index.unique {
        statement.unique();
    }
    statement
}

fn foreign_key_statement(
    table_name: &'static str,
    foreign_key: &ForeignKeySpec,
) -> ForeignKeyCreateStatement {
    let mut statement = ForeignKey::create();
    statement
        .name(foreign_key.name)
        .from(
            SchemaIden::new(table_name),
            SchemaIden::new(foreign_key.column),
        )
        .to(
            SchemaIden::new(foreign_key.to_table),
            SchemaIden::new(foreign_key.to_column),
        )
        .on_delete(foreign_key_action(foreign_key.on_delete));
    statement
}

fn foreign_key_action(action: ForeignKeyActionSpec) -> ForeignKeyAction {
    match action {
        ForeignKeyActionSpec::Cascade => ForeignKeyAction::Cascade,
        ForeignKeyActionSpec::SetNull => ForeignKeyAction::SetNull,
    }
}

pub async fn reset_metadata_schema(db: &DatabaseConnection) -> Result<(), DbErr> {
    let backend = db.get_database_backend();
    let tables = schema_contract::metadata_reset_tables().join(", ");
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
    for table in schema_contract::catalog_row_tables() {
        if !manager.has_table(table.name).await? {
            continue;
        }
        let row = db
            .query_one(Statement::from_string(
                backend,
                format!("SELECT 1 FROM {} LIMIT 1", table.name),
            ))
            .await?;
        if row.is_some() {
            return Ok(true);
        }
    }

    Ok(false)
}

async fn metadata_schema_drift(
    db: &DatabaseConnection,
    manager: &SchemaManager<'_>,
) -> Result<Option<String>, DbErr> {
    for table in schema_contract::schema_tables() {
        if !manager.has_table(table.name).await? {
            return Ok(Some(format!("missing table {}", table.name)));
        }
        for column in table.column_names() {
            if !manager.has_column(table.name, column).await? {
                return Ok(Some(format!("missing column {}.{column}", table.name)));
            }
        }
        let actual_columns = metadata_table_columns(db, table.name).await?;
        let expected_columns = table.column_names().collect::<Vec<_>>();
        if actual_columns.len() != expected_columns.len() {
            let expected = expected_columns.join(", ");
            let actual = actual_columns.join(", ");
            return Ok(Some(format!(
                "column mismatch {}: expected [{expected}], found [{actual}]",
                table.name
            )));
        }
    }
    Ok(None)
}

async fn metadata_table_columns(
    db: &DatabaseConnection,
    table_name: &str,
) -> Result<Vec<String>, DbErr> {
    let rows = db
        .query_all(Statement::from_string(
            db.get_database_backend(),
            format!(
                "SELECT column_name FROM information_schema.columns WHERE table_schema = current_schema() AND table_name = '{}' ORDER BY ordinal_position",
                table_name.replace('\'', "''")
            ),
        ))
        .await?;
    rows.into_iter().map(column_name_from_row).collect()
}

fn column_name_from_row(row: QueryResult) -> Result<String, DbErr> {
    row.try_get("", "column_name")
        .map_err(|error| DbErr::Custom(format!("reading metadata column name failed: {error}")))
}

async fn metadata_schema_has_duplicate_user_emails(
    db: &DatabaseConnection,
    manager: &SchemaManager<'_>,
) -> Result<bool, DbErr> {
    if !manager.has_table(schema_contract::auth::USERS.name).await?
        || !manager
            .has_column(
                schema_contract::auth::USERS.name,
                schema_contract::auth::USER_EMAIL,
            )
            .await?
    {
        return Ok(false);
    }

    let row = db
        .query_one(Statement::from_string(
            db.get_database_backend(),
            format!(
                "SELECT {email} FROM {users} WHERE {email} <> '' GROUP BY {email} HAVING COUNT(*) > 1 LIMIT 1",
                users = schema_contract::auth::USERS.name,
                email = schema_contract::auth::USER_EMAIL,
            ),
        ))
        .await?;
    Ok(row.is_some())
}

fn is_destructive_pre_alpha_reset_drift(drift: &str) -> bool {
    drift.starts_with("missing table ")
        || drift.starts_with("missing column ")
        || drift.starts_with("column mismatch ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn creates_users_table_before_user_email_index() {
        use sea_orm::{DbBackend, MockDatabase, MockExecResult};

        let db = MockDatabase::new(DbBackend::Postgres)
            .append_exec_results(vec![MockExecResult::default(); 64])
            .into_connection();
        let manager = SchemaManager::new(&db);

        ensure_schema_tables(&manager).await.unwrap();

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
        assert!(is_destructive_pre_alpha_reset_drift(
            "column mismatch scope_source_blob_cleanup_jobs: expected [object_key], found [object_key, line_count]"
        ));
    }
}
