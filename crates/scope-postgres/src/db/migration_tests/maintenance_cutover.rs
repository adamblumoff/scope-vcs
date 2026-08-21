use super::*;
use crate::db::{
    apply_maintenance_migrations, connect_postgres_worker_store, connect_writer_database,
    terminate_metadata_writer_sessions, verify_writer_fence_available,
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
async fn truthful_log_truncation_cutover_requires_maintenance() {
    let (_target, db, _lease) = isolated_database().await;
    migrations::Migrator::up(db.as_ref(), Some(17))
        .await
        .unwrap();

    let plan = migrations::plan(db.as_ref()).await.unwrap();

    assert_eq!(plan.pending.len(), 9);
    assert_eq!(plan.pending[0].name, "m0018_truthful_run_log_truncation");
    assert_eq!(plan.pending[0].impact, MigrationImpact::MaintenanceRequired);
    assert_eq!(plan.pending[1].name, "m0019_run_attempt_cache_observations");
    assert_eq!(plan.pending[1].impact, MigrationImpact::Online);
    assert_eq!(plan.pending[2].name, "m0020_cloud_execution");
    assert_eq!(plan.pending[2].impact, MigrationImpact::MaintenanceRequired);
    assert_eq!(plan.pending[3].name, "m0021_cache_service_cutover");
    assert_eq!(plan.pending[3].impact, MigrationImpact::MaintenanceRequired);
    assert_eq!(plan.pending[4].name, "m0022_git_pack_spans");
    assert_eq!(plan.pending[4].impact, MigrationImpact::MaintenanceRequired);
    assert_eq!(plan.pending[5].name, "m0023_logical_run_sources");
    assert_eq!(plan.pending[5].impact, MigrationImpact::MaintenanceRequired);
    assert_eq!(plan.pending[6].name, "m0024_git_compaction_scheduler");
    assert_eq!(plan.pending[6].impact, MigrationImpact::Online);
    assert_eq!(plan.pending[7].name, "m0025_visibility_change_sets");
    assert_eq!(plan.pending[7].impact, MigrationImpact::MaintenanceRequired);
    assert_eq!(plan.pending[8].name, "m0026_repository_landing_files");
    assert_eq!(plan.pending[8].impact, MigrationImpact::MaintenanceRequired);
}

#[tokio::test]
async fn cache_service_cutover_requires_maintenance() {
    let (_target, db, _lease) = isolated_database().await;
    migrations::Migrator::up(db.as_ref(), Some(20))
        .await
        .unwrap();

    let plan = migrations::plan(db.as_ref()).await.unwrap();

    assert_eq!(plan.pending.len(), 6);
    assert_eq!(plan.pending[0].name, "m0021_cache_service_cutover");
    assert_eq!(plan.pending[0].impact, MigrationImpact::MaintenanceRequired);
    assert_eq!(plan.pending[1].name, "m0022_git_pack_spans");
    assert_eq!(plan.pending[1].impact, MigrationImpact::MaintenanceRequired);
    assert_eq!(plan.pending[2].name, "m0023_logical_run_sources");
    assert_eq!(plan.pending[2].impact, MigrationImpact::MaintenanceRequired);
    assert_eq!(plan.pending[3].name, "m0024_git_compaction_scheduler");
    assert_eq!(plan.pending[3].impact, MigrationImpact::Online);
    assert_eq!(plan.pending[4].name, "m0025_visibility_change_sets");
    assert_eq!(plan.pending[4].impact, MigrationImpact::MaintenanceRequired);
    assert_eq!(plan.pending[5].name, "m0026_repository_landing_files");
    assert_eq!(plan.pending[5].impact, MigrationImpact::MaintenanceRequired);
}

#[tokio::test]
async fn compaction_scheduler_is_an_online_additive_migration() {
    let (_target, db, _lease) = isolated_database().await;
    migrations::Migrator::up(db.as_ref(), Some(23))
        .await
        .unwrap();

    let plan = migrations::plan(db.as_ref()).await.unwrap();
    assert_eq!(plan.pending.len(), 3);
    assert_eq!(plan.pending[0].name, "m0024_git_compaction_scheduler");
    assert_eq!(plan.pending[0].impact, MigrationImpact::Online);
    assert_eq!(plan.pending[1].name, "m0025_visibility_change_sets");
    assert_eq!(plan.pending[1].impact, MigrationImpact::MaintenanceRequired);
    assert_eq!(plan.pending[2].name, "m0026_repository_landing_files");
    assert_eq!(plan.pending[2].impact, MigrationImpact::MaintenanceRequired);

    let error = migrations::apply_online(db.as_ref()).await.unwrap_err();
    assert!(error.to_string().contains("m0025_visibility_change_sets"));
    assert!(relation_exists(db.as_ref(), "scope_git_compaction_jobs").await);
}

#[tokio::test]
async fn truthful_log_cutover_fences_protocol_six_runners() {
    let (_target, db, _lease) = isolated_database().await;
    migrations::Migrator::up(db.as_ref(), Some(17))
        .await
        .unwrap();
    db.execute_unprepared(
        "INSERT INTO scope_users (id, handle, email, email_verified)
         VALUES ('user_v7', 'v7-owner', 'v7@scope.test', TRUE);
         INSERT INTO scope_runners (
             id, owner_user_id, secret_hash, version, protocol_version,
             capabilities, max_concurrent_jobs, enabled, created_at_unix,
             last_seen_at_unix
         ) VALUES (
             'runner_v6', 'user_v7', repeat('a', 64), '0.1.0', 6,
             '{\"operating_system\":\"linux\",\"architecture\":\"amd64\",\"container_engine\":\"docker\"}'::jsonb,
             1, TRUE, 1, NULL
         );",
    )
    .await
    .unwrap();

    migrations::Migrator::up(db.as_ref(), Some(1))
        .await
        .unwrap();

    let cutover = db
        .query_one(Statement::from_string(
            DatabaseBackend::Postgres,
            "SELECT state FROM scope_runner_protocol_cutover WHERE key = 'current'".to_string(),
        ))
        .await
        .unwrap()
        .unwrap()
        .try_get::<String>("", "state")
        .unwrap();
    assert_eq!(cutover, "v7-open");
    let enabled = db
        .query_one(Statement::from_string(
            DatabaseBackend::Postgres,
            "SELECT enabled FROM scope_runners WHERE id = 'runner_v6'".to_string(),
        ))
        .await
        .unwrap()
        .unwrap()
        .try_get::<bool>("", "enabled")
        .unwrap();
    assert!(!enabled);
    assert!(
        db.execute_unprepared("UPDATE scope_runners SET enabled = TRUE WHERE id = 'runner_v6'")
            .await
            .is_err()
    );
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

    let fence_error = verify_writer_fence_available(database_url.clone())
        .await
        .unwrap_err();
    assert!(fence_error.to_string().contains("writer still holds"));
    let migration_error = apply_maintenance_migrations(database_url.clone())
        .await
        .unwrap_err();
    assert!(migration_error.to_string().contains("writer still holds"));
    assert!(relation_exists(db.as_ref(), "scope_request_change_blocks").await);

    assert_eq!(
        terminate_metadata_writer_sessions(database_url.clone())
            .await
            .unwrap(),
        1
    );
    writer.close().await.unwrap();
    verify_writer_fence_available(database_url.clone())
        .await
        .unwrap();
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
