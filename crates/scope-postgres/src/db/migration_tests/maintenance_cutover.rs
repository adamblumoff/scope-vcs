use super::*;
use crate::db::{
    apply_maintenance_migrations, connect_postgres_worker_store, connect_writer_database,
};

#[tokio::test]
async fn ordinary_startup_refuses_pending_maintenance_migration() {
    let (target, db, _lease) = isolated_database().await;
    migrations::Migrator::up(db.as_ref(), Some(11))
        .await
        .unwrap();

    let error = migrations::apply_online(db.as_ref()).await.unwrap_err();

    assert!(error.to_string().contains("m0012_request_revisions"));
    assert!(relation_exists(db.as_ref(), "scope_request_change_blocks").await);
    let plan = migrations::plan(db.as_ref()).await.unwrap();
    assert_eq!(plan.pending[0].name, "m0012_request_revisions");
    assert_eq!(plan.pending[0].impact, MigrationImpact::MaintenanceRequired);

    let worker_error = match connect_postgres_worker_store(target.schema_database_url()).await {
        Ok(_) => panic!("worker must refuse a pending maintenance migration"),
        Err(error) => error,
    };
    assert!(
        worker_error
            .to_string()
            .contains("does not match this binary")
    );
}

#[tokio::test]
async fn migration_inventory_classifies_data_rewrite_and_contract_cutover_together() {
    let (_target, db, _lease) = isolated_database().await;
    migrations::Migrator::up(db.as_ref(), Some(10))
        .await
        .unwrap();

    let plan = migrations::plan(db.as_ref()).await.unwrap();

    assert_eq!(plan.pending[0].name, "m0011_compact_request_started_events");
    assert_eq!(plan.pending[0].impact, MigrationImpact::MaintenanceRequired);
    assert_eq!(plan.pending[1].name, "m0012_request_revisions");
    assert_eq!(plan.pending[1].impact, MigrationImpact::MaintenanceRequired);
}

#[tokio::test]
async fn maintenance_cutover_refuses_a_writer_after_its_pool_reconnects() {
    let (target, db, _lease) = isolated_database().await;
    migrations::Migrator::up(db.as_ref(), Some(11))
        .await
        .unwrap();
    let database_url = target.schema_database_url();
    let writer = connect_writer_database(&database_url).await.unwrap();

    let writer_pid = writer
        .query_one(Statement::from_string(
            DatabaseBackend::Postgres,
            "SELECT pg_backend_pid() AS pid".to_string(),
        ))
        .await
        .unwrap()
        .unwrap()
        .try_get::<i32>("", "pid")
        .unwrap();
    db.execute(Statement::from_sql_and_values(
        DatabaseBackend::Postgres,
        "SELECT pg_terminate_backend($1)",
        [writer_pid.into()],
    ))
    .await
    .unwrap();
    writer.ping().await.unwrap();

    let error = apply_maintenance_migrations(database_url.clone())
        .await
        .unwrap_err();
    assert!(error.to_string().contains("writer still holds"));
    assert!(relation_exists(db.as_ref(), "scope_request_change_blocks").await);

    writer.close().await.unwrap();
    apply_maintenance_migrations(database_url).await.unwrap();
    migrations::assert_exact_state(db.as_ref()).await.unwrap();
    assert!(!relation_exists(db.as_ref(), "scope_request_change_blocks").await);
    assert!(relation_exists(db.as_ref(), "scope_request_revisions").await);

    let worker_store = connect_postgres_worker_store(target.schema_database_url())
        .await
        .unwrap();
    worker_store.admin().readiness_check().await.unwrap();

    db.execute_unprepared(
        "
            INSERT INTO seaql_migrations (version, applied_at)
            VALUES ('m9999_unknown', 0)
        ",
    )
    .await
    .unwrap();
    assert!(worker_store.admin().readiness_check().await.is_err());
}
