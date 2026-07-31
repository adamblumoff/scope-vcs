use super::migration_tests::{isolated_database, relation_exists};
use sea_orm::{ConnectionTrait, DatabaseBackend, DynIden, Statement};
use sea_orm_migration::{
    MigrationName, MigrationTrait, MigratorTrait, SchemaManager, sea_query::IntoIden,
};

struct TransformMigrator;
struct TransformMigration;
struct FailingTransformMigrator;
struct FailingTransformMigration;

#[sea_orm_migration::async_trait::async_trait]
impl MigratorTrait for TransformMigrator {
    fn migrations() -> Vec<Box<dyn MigrationTrait>> {
        vec![Box::new(TransformMigration)]
    }

    fn migration_table_name() -> DynIden {
        "seaql_test_transform_migrations".into_iden()
    }
}

impl MigrationName for TransformMigration {
    fn name(&self) -> &str {
        "m0001_transform_legacy_values"
    }
}

#[sea_orm_migration::async_trait::async_trait]
impl MigrationTrait for TransformMigration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), sea_orm::DbErr> {
        manager
            .get_connection()
            .execute_unprepared(
                "
                    CREATE TABLE scope_test_transformed_values (
                        id text PRIMARY KEY,
                        doubled_value bigint NOT NULL
                    );
                    INSERT INTO scope_test_transformed_values (id, doubled_value)
                    SELECT id, legacy_value * 2
                    FROM scope_test_legacy_values;
                    DO $$
                    BEGIN
                        IF (
                            SELECT count(*) FROM scope_test_transformed_values
                        ) <> (
                            SELECT count(*) FROM scope_test_legacy_values
                        ) THEN
                            RAISE EXCEPTION 'transform row count mismatch';
                        END IF;
                    END $$;
                    DROP TABLE scope_test_legacy_values;
                ",
            )
            .await?;
        Ok(())
    }
}

#[sea_orm_migration::async_trait::async_trait]
impl MigratorTrait for FailingTransformMigrator {
    fn migrations() -> Vec<Box<dyn MigrationTrait>> {
        vec![Box::new(FailingTransformMigration)]
    }

    fn migration_table_name() -> DynIden {
        "seaql_test_failing_transform_migrations".into_iden()
    }
}

impl MigrationName for FailingTransformMigration {
    fn name(&self) -> &str {
        "m0001_failing_transform"
    }
}

#[sea_orm_migration::async_trait::async_trait]
impl MigrationTrait for FailingTransformMigration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), sea_orm::DbErr> {
        manager
            .get_connection()
            .execute_unprepared(
                "
                    CREATE TABLE scope_test_failed_destination (
                        id text PRIMARY KEY,
                        doubled_value bigint NOT NULL
                    );
                    INSERT INTO scope_test_failed_destination (id, doubled_value)
                    SELECT id, legacy_value * 2
                    FROM scope_test_legacy_values;
                    DO $$
                    BEGIN
                        RAISE EXCEPTION 'injected transform validation failure';
                    END $$;
                    DROP TABLE scope_test_legacy_values;
                ",
            )
            .await?;
        Ok(())
    }
}

#[tokio::test]
async fn migration_harness_transforms_rows_and_retires_source_schema() {
    let (_target, db, _lease) = isolated_database().await;
    db.execute_unprepared(
        "
            CREATE TABLE scope_test_legacy_values (
                id text PRIMARY KEY,
                legacy_value bigint NOT NULL
            );
            INSERT INTO scope_test_legacy_values (id, legacy_value)
            VALUES ('one', 4), ('two', 9);
        ",
    )
    .await
    .unwrap();

    TransformMigrator::up(db.as_ref(), None).await.unwrap();

    let values = db
        .query_all(Statement::from_string(
            DatabaseBackend::Postgres,
            "
                SELECT id, doubled_value
                FROM scope_test_transformed_values
                ORDER BY id
            "
            .to_string(),
        ))
        .await
        .unwrap()
        .into_iter()
        .map(|row| {
            (
                row.try_get::<String>("", "id").unwrap(),
                row.try_get::<i64>("", "doubled_value").unwrap(),
            )
        })
        .collect::<Vec<_>>();
    assert_eq!(values, [("one".to_string(), 8), ("two".to_string(), 18)]);
    assert!(!relation_exists(db.as_ref(), "scope_test_legacy_values").await);
    assert!(relation_exists(db.as_ref(), "scope_test_transformed_values").await);
}

#[tokio::test]
async fn failed_transform_rolls_back_schema_data_and_ledger() {
    let (_target, db, _lease) = isolated_database().await;
    db.execute_unprepared(
        "
            CREATE TABLE scope_test_legacy_values (
                id text PRIMARY KEY,
                legacy_value bigint NOT NULL
            );
            INSERT INTO scope_test_legacy_values (id, legacy_value)
            VALUES ('one', 4);
        ",
    )
    .await
    .unwrap();

    let error = FailingTransformMigrator::up(db.as_ref(), None)
        .await
        .unwrap_err();

    assert!(
        error
            .to_string()
            .contains("injected transform validation failure")
    );
    assert!(relation_exists(db.as_ref(), "scope_test_legacy_values").await);
    assert!(!relation_exists(db.as_ref(), "scope_test_failed_destination").await);
    assert!(!relation_exists(db.as_ref(), "seaql_test_failing_transform_migrations").await);
}
