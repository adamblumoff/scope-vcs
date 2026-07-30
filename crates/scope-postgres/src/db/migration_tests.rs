use super::{
    TestDatabaseTarget, connect_postgres_worker_store_with_schema_wait,
    test_support::{TestSchemaLease, connect_isolated_test_database},
};
use crate::migrations;
use sea_orm::{ConnectionTrait, Database, DatabaseBackend, DatabaseConnection, DynIden, Statement};
use sea_orm_migration::{
    MigrationName, MigrationTrait, MigratorTrait, SchemaManager, sea_query::IntoIden,
};
use std::{sync::Arc, time::Duration};

const V6_SCHEMA: &str = include_str!("../migrations/v6.sql");
const RETIRED_V6_TABLES: &[&str] = &[
    "scope_repository_git_clone_tokens",
    "scope_repository_git_snapshots",
    "scope_repository_settings",
    "scope_source_blob_cleanup_jobs",
];

async fn isolated_database() -> (
    TestDatabaseTarget,
    Arc<DatabaseConnection>,
    Arc<TestSchemaLease>,
) {
    let target = TestDatabaseTarget::required().unwrap();
    let (db, lease) = connect_isolated_test_database(&target).await.unwrap();
    (target, db, lease)
}

async fn initialize_ready_v6(db: &DatabaseConnection) {
    db.execute_unprepared(V6_SCHEMA).await.unwrap();
    db.execute_unprepared(
        "
            INSERT INTO scope_metadata_schema (key, version, deploy_revision, ready)
            VALUES ('current', 6, 'legacy-v6', TRUE)
        ",
    )
    .await
    .unwrap();
}

async fn add_retired_v6_tables(db: &DatabaseConnection) {
    db.execute_unprepared(
        "
            CREATE TABLE scope_repository_git_clone_tokens (id text PRIMARY KEY);
            CREATE TABLE scope_repository_git_snapshots (id text PRIMARY KEY);
            CREATE TABLE scope_repository_settings (id text PRIMARY KEY);
            CREATE TABLE scope_source_blob_cleanup_jobs (id text PRIMARY KEY);
            INSERT INTO scope_source_blob_cleanup_jobs (id) VALUES ('obsolete');
        ",
    )
    .await
    .unwrap();
}

async fn relation_exists(db: &DatabaseConnection, relation: &str) -> bool {
    db.query_one(Statement::from_sql_and_values(
        DatabaseBackend::Postgres,
        "SELECT to_regclass($1) IS NOT NULL AS exists",
        [relation.into()],
    ))
    .await
    .unwrap()
    .unwrap()
    .try_get::<bool>("", "exists")
    .unwrap()
}

async fn applied_versions(db: &DatabaseConnection) -> Vec<String> {
    db.query_all(Statement::from_string(
        DatabaseBackend::Postgres,
        "SELECT version FROM seaql_migrations ORDER BY version".to_string(),
    ))
    .await
    .unwrap()
    .into_iter()
    .map(|row| row.try_get::<String>("", "version").unwrap())
    .collect()
}

async fn representative_business_snapshot(db: &DatabaseConnection) -> String {
    db.query_one(Statement::from_string(
        DatabaseBackend::Postgres,
        "
            SELECT jsonb_build_object(
                'users', (SELECT jsonb_agg(to_jsonb(item) ORDER BY id) FROM scope_users item),
                'auth', (
                    SELECT jsonb_agg(to_jsonb(item) ORDER BY provider, subject)
                    FROM scope_auth_identities item
                ),
                'repositories', (
                    SELECT jsonb_agg(to_jsonb(item) ORDER BY id)
                    FROM scope_repositories item
                ),
                'requests', (
                    SELECT jsonb_agg(to_jsonb(item) ORDER BY id)
                    FROM scope_requests item
                ),
                'credit_accounts', (
                    SELECT jsonb_agg(to_jsonb(item) ORDER BY user_id)
                    FROM scope_user_credit_accounts item
                ),
                'credit_entries', (
                    SELECT jsonb_agg(to_jsonb(item) ORDER BY id)
                    FROM scope_credit_ledger_entries item
                ),
                'workflow_revisions', (
                    SELECT jsonb_agg(to_jsonb(item) ORDER BY digest)
                    FROM scope_workflow_revisions item
                ),
                'runs', (
                    SELECT jsonb_agg(to_jsonb(item) ORDER BY id)
                    FROM scope_runs item
                ),
                'outbox', (
                    SELECT jsonb_agg(to_jsonb(item) ORDER BY id)
                    FROM scope_outbox_jobs item
                ),
                'projections', (
                    SELECT jsonb_agg(to_jsonb(item) ORDER BY repo_id, source, audience)
                    FROM scope_projection_read_models item
                )
            )::text AS value
        "
        .to_string(),
    ))
    .await
    .unwrap()
    .unwrap()
    .try_get::<String>("", "value")
    .unwrap()
}

#[tokio::test]
async fn fresh_database_reaches_exact_latest_schema() {
    let (_target, db, _lease) = isolated_database().await;

    migrations::apply(db.as_ref()).await.unwrap();

    migrations::assert_exact_state(db.as_ref()).await.unwrap();
    assert_eq!(
        applied_versions(db.as_ref()).await,
        [
            "m0001_adopt_v6",
            "m0002_retire_reset_schema",
            "m0003_structured_run_attempts",
        ]
    );
    assert!(!relation_exists(db.as_ref(), "scope_metadata_schema").await);
    assert!(!relation_exists(db.as_ref(), "scope_metadata_reset_events").await);
    let scope_table_count = db
        .query_one(Statement::from_string(
            DatabaseBackend::Postgres,
            "
                SELECT count(*) AS count
                FROM pg_tables
                WHERE schemaname = current_schema()
                  AND left(tablename, 6) = 'scope_'
            "
            .to_string(),
        ))
        .await
        .unwrap()
        .unwrap()
        .try_get::<i64>("", "count")
        .unwrap();
    assert_eq!(scope_table_count, 41);
}

#[tokio::test]
async fn populated_v6_is_adopted_without_changing_business_rows() {
    let (_target, db, _lease) = isolated_database().await;
    initialize_ready_v6(db.as_ref()).await;
    db.execute_unprepared(
        "
            INSERT INTO scope_users (id, handle, email, email_verified)
            VALUES ('user_legacy', 'legacy', 'legacy@scope.test', TRUE);
            INSERT INTO scope_auth_identities (provider, subject, user_id)
            VALUES ('clerk', 'legacy-subject', 'user_legacy');
            INSERT INTO scope_repositories (
                id, owner_handle, name, owner_user_id, publication_state,
                default_visibility, change_version, repo_config, policy
            )
            VALUES (
                'repo_legacy', 'legacy', 'repo', 'user_legacy', 'Published',
                'Private', 1, '{}'::jsonb, '{}'::jsonb
            );
            INSERT INTO scope_requests (
                id, repo_id, name, author_user_id, author_role, audience,
                base_main_oid, head_oid, git_snapshot, title, description_markdown,
                state, activity_version, ready_queue_version, current_stake_credits,
                first_ready_at_unix, ready_at_unix, held_at_unix, held_by_user_id,
                assessment_outcome, assessment_body_markdown, assessed_at_unix,
                assessed_by_user_id, completed_at_unix, completed_by_user_id,
                merged_at_unix, merged_by_user_id, merged_head_oid, merged_main_oid,
                created_at_unix, updated_at_unix
            )
            VALUES (
                'request_legacy', 'repo_legacy', 'legacy-change', 'user_legacy',
                'Owner', 'Private', repeat('d', 40), repeat('e', 40), NULL,
                'Legacy request', 'Preserve this request', 'Working', 0, NULL, 0,
                NULL, NULL, NULL, NULL, NULL, NULL, NULL, NULL, NULL, NULL,
                NULL, NULL, NULL, NULL, 1, 1
            );
            INSERT INTO scope_user_credit_accounts (user_id, balance_credits)
            VALUES ('user_legacy', 100);
            INSERT INTO scope_credit_ledger_entries (
                id, user_id, request_id, kind, amount_credits, created_at_unix
            )
            VALUES ('credit_legacy', 'user_legacy', NULL, 'StarterGrant', 100, 1);
            INSERT INTO scope_workflow_revisions (digest, definition, created_at_unix)
            VALUES (repeat('a', 64), '{}'::jsonb, 1);
            INSERT INTO scope_runs (
                id, idempotency_key, repo_id, workflow_path,
                workflow_revision_digest, trigger, requested_by_user_id, source,
                pinned_container_image, desired_runner_name, state,
                cancellation_requested, last_attempt_number, current_attempt_id,
                created_at_unix, updated_at_unix, completed_at_unix
            )
            VALUES (
                'run_legacy', 'manual:legacy', 'repo_legacy', '.scope/workflow.yml',
                repeat('a', 64), 'manual', 'user_legacy',
                jsonb_build_object(
                    'kind', 'ephemeral-git-bundle',
                    'object', jsonb_build_object(
                        'sha256', repeat('b', 64),
                        'git_oid', repeat('c', 40)
                    )
                ),
                NULL, NULL, 'queued', FALSE, 0, NULL, 1, 1, NULL
            );
            INSERT INTO scope_outbox_jobs (
                id, idempotency_key, kind, repo_id, repo_version, payload, state,
                attempts, next_run_at_unix, lease_owner, lease_expires_at_unix,
                last_error, created_at_unix, updated_at_unix, completed_at_unix
            )
            VALUES (
                'outbox_legacy', 'legacy:1', 'projection', 'repo_legacy', 1,
                '{}'::jsonb, 'ready', 0, 1, NULL, NULL, NULL, 1, 1, NULL
            );
            INSERT INTO scope_projection_read_models (
                repo_id, repo_version, source, audience, rebuilt_at_unix, file_count
            )
            VALUES ('repo_legacy', 1, 'live', 'private', 1, 0);
            INSERT INTO scope_metadata_locks (key) VALUES ('repository:legacy/repo');
        ",
    )
    .await
    .unwrap();
    let before = representative_business_snapshot(db.as_ref()).await;

    migrations::apply(db.as_ref()).await.unwrap();

    let after = representative_business_snapshot(db.as_ref()).await;
    assert_eq!(after, before);
    assert!(
        db.query_one(Statement::from_string(
            DatabaseBackend::Postgres,
            "
                SELECT 1
                FROM scope_metadata_locks
                WHERE key = 'repository:legacy/repo'
            "
            .to_string(),
        ))
        .await
        .unwrap()
        .is_some()
    );
    assert!(!relation_exists(db.as_ref(), "scope_metadata_schema").await);
    assert!(!relation_exists(db.as_ref(), "scope_metadata_reset_events").await);
}

#[tokio::test]
async fn structured_attempt_migration_preserves_runs_and_replaces_execution_state() {
    let (_target, db, _lease) = isolated_database().await;
    initialize_ready_v6(db.as_ref()).await;
    db.execute_unprepared(
        "
            INSERT INTO scope_users (id, handle, email, email_verified)
            VALUES ('user_run', 'run-owner', 'run-owner@scope.test', TRUE);
            INSERT INTO scope_repositories (
                id, owner_handle, name, owner_user_id, publication_state,
                default_visibility, change_version, repo_config, policy
            )
            VALUES (
                'repo_run', 'run-owner', 'repo', 'user_run', 'Published',
                'Private', 1, '{}'::jsonb, '{}'::jsonb
            );
            INSERT INTO scope_runners (
                id, owner_user_id, secret_hash, version, protocol_version,
                capabilities, enabled, created_at_unix, last_seen_at_unix
            )
            VALUES (
                'runner_run', 'user_run', repeat('f', 64), '0.1.0', 2,
                '{}'::jsonb, TRUE, 1, 2
            );
            INSERT INTO scope_runner_grants (
                repo_id, runner_id, name, granted_by_user_id,
                created_at_unix, revoked_at_unix
            )
            VALUES ('repo_run', 'runner_run', 'linux', 'user_run', 1, NULL);
            INSERT INTO scope_workflow_revisions (digest, definition, created_at_unix)
            VALUES (repeat('a', 64), '{}'::jsonb, 1);
            INSERT INTO scope_runs (
                id, idempotency_key, repo_id, workflow_path,
                workflow_revision_digest, trigger, requested_by_user_id, source,
                pinned_container_image, desired_runner_name, state,
                cancellation_requested, last_attempt_number, current_attempt_id,
                created_at_unix, updated_at_unix, completed_at_unix
            )
            VALUES (
                'run_active', 'manual:active', 'repo_run', '.scope/workflow.yml',
                repeat('a', 64), 'manual', 'user_run',
                jsonb_build_object(
                    'kind', 'ephemeral-git-bundle',
                    'object', jsonb_build_object(
                        'sha256', repeat('b', 64),
                        'git_oid', repeat('c', 40)
                    )
                ),
                NULL, 'linux', 'queued', FALSE, 0, NULL, 1, 1, NULL
            );
            INSERT INTO scope_run_attempts (
                id, run_id, number, runner_id, token_hash,
                token_expires_at_unix, state, lease_expires_at_unix,
                last_heartbeat_at_unix, created_at_unix, started_at_unix,
                completed_at_unix, exit_code, log_bytes, logs_truncated
            )
            VALUES (
                'attempt_active', 'run_active', 1, 'runner_run', repeat('d', 64),
                100, 'running', 100, 2, 1, 2, NULL, NULL, 4, FALSE
            );
            UPDATE scope_runs
            SET state = 'running',
                last_attempt_number = 1,
                current_attempt_id = 'attempt_active',
                updated_at_unix = 2
            WHERE id = 'run_active';
            INSERT INTO scope_run_logs (
                run_id, attempt_id, sequence, text, created_at_unix
            )
            VALUES ('run_active', 'attempt_active', 1, 'test', 2);
        ",
    )
    .await
    .unwrap();

    migrations::apply(db.as_ref()).await.unwrap();

    let run = db
        .query_one(Statement::from_string(
            DatabaseBackend::Postgres,
            "
                SELECT state, cancellation_requested, last_attempt_number,
                       current_attempt_id, completed_at_unix
                FROM scope_runs
                WHERE id = 'run_active'
            "
            .to_string(),
        ))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(run.try_get::<String>("", "state").unwrap(), "queued");
    assert!(!run.try_get::<bool>("", "cancellation_requested").unwrap());
    assert_eq!(run.try_get::<i32>("", "last_attempt_number").unwrap(), 0);
    assert!(
        run.try_get::<Option<String>>("", "current_attempt_id")
            .unwrap()
            .is_none()
    );
    assert!(
        run.try_get::<Option<i64>>("", "completed_at_unix")
            .unwrap()
            .is_none()
    );
    for table in [
        "scope_run_attempts",
        "scope_run_attempt_steps",
        "scope_run_logs",
    ] {
        let count = db
            .query_one(Statement::from_string(
                DatabaseBackend::Postgres,
                format!("SELECT count(*) AS count FROM {table}"),
            ))
            .await
            .unwrap()
            .unwrap()
            .try_get::<i64>("", "count")
            .unwrap();
        assert_eq!(count, 0, "{table} should start empty after the reset");
    }
    for index in [
        "idx_scope_run_attempts_active",
        "idx_scope_run_attempts_runner",
        "idx_scope_run_attempts_expiring",
        "idx_scope_run_logs_run_position",
        "idx_scope_run_logs_step_position",
    ] {
        assert!(relation_exists(db.as_ref(), index).await, "missing {index}");
    }
    let constraints = db
        .query_one(Statement::from_string(
            DatabaseBackend::Postgres,
            "
                SELECT
                    count(*) FILTER (
                        WHERE conname IN (
                            'fk_scope_run_attempts_run',
                            'fk_scope_run_attempts_runner',
                            'fk_scope_run_attempt_steps_attempt',
                            'fk_scope_run_logs_run',
                            'fk_scope_run_logs_step',
                            'fk_scope_runs_current_attempt'
                        )
                    ) AS foreign_keys,
                    bool_or(
                        conname = 'scope_run_attempts_values' AND
                        pg_get_constraintdef(oid) LIKE '%last_heartbeat_at_unix < lease_expires_at_unix%'
                    ) AS lease_check,
                    bool_or(
                        conname = 'scope_run_logs_values' AND
                        pg_get_constraintdef(oid) LIKE '%octet_length(text)%'
                    ) AS byte_check
                FROM pg_constraint
                WHERE connamespace = current_schema()::regnamespace
            "
            .to_string(),
        ))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(constraints.try_get::<i64>("", "foreign_keys").unwrap(), 6);
    assert!(constraints.try_get::<bool>("", "lease_check").unwrap());
    assert!(constraints.try_get::<bool>("", "byte_check").unwrap());
}

#[tokio::test]
async fn structurally_drifted_v6_is_refused_without_stamping_migration_state() {
    let (_target, db, _lease) = isolated_database().await;
    initialize_ready_v6(db.as_ref()).await;
    db.execute_unprepared("ALTER TABLE scope_users DROP COLUMN email_verified")
        .await
        .unwrap();

    let error = migrations::apply(db.as_ref()).await.unwrap_err();

    assert!(error.to_string().contains("column fingerprint"));
    assert!(!relation_exists(db.as_ref(), "seaql_migrations").await);
    assert!(relation_exists(db.as_ref(), "scope_metadata_schema").await);
}

#[tokio::test]
async fn production_v6_with_retired_tables_is_adopted_and_cleaned_up() {
    let (_target, db, _lease) = isolated_database().await;
    initialize_ready_v6(db.as_ref()).await;
    add_retired_v6_tables(db.as_ref()).await;

    migrations::apply(db.as_ref()).await.unwrap();

    for table in RETIRED_V6_TABLES {
        assert!(
            !relation_exists(db.as_ref(), table).await,
            "{table} should be retired during v6 adoption"
        );
    }
    migrations::assert_exact_state(db.as_ref()).await.unwrap();
}

#[tokio::test]
async fn rejected_v6_fingerprint_rolls_back_retired_table_cleanup() {
    let (_target, db, _lease) = isolated_database().await;
    initialize_ready_v6(db.as_ref()).await;
    add_retired_v6_tables(db.as_ref()).await;
    db.execute_unprepared("ALTER TABLE scope_users DROP COLUMN email_verified")
        .await
        .unwrap();

    let error = migrations::apply(db.as_ref()).await.unwrap_err();

    assert!(error.to_string().contains("column fingerprint"));
    assert!(!relation_exists(db.as_ref(), "seaql_migrations").await);
    for table in RETIRED_V6_TABLES {
        assert!(
            relation_exists(db.as_ref(), table).await,
            "{table} should be restored when v6 adoption rolls back"
        );
    }
    let obsolete_rows = db
        .query_one(Statement::from_string(
            DatabaseBackend::Postgres,
            "
                SELECT count(*) AS count
                FROM scope_source_blob_cleanup_jobs
                WHERE id = 'obsolete'
            "
            .to_string(),
        ))
        .await
        .unwrap()
        .unwrap()
        .try_get::<i64>("", "count")
        .unwrap();
    assert_eq!(obsolete_rows, 1);
}

#[tokio::test]
async fn unknown_v6_table_is_refused_without_deleting_it() {
    let (_target, db, _lease) = isolated_database().await;
    initialize_ready_v6(db.as_ref()).await;
    db.execute_unprepared(
        "
            CREATE TABLE scope_unexpected_production_table (
                id text PRIMARY KEY
            );
            INSERT INTO scope_unexpected_production_table (id) VALUES ('keep');
        ",
    )
    .await
    .unwrap();

    let error = migrations::apply(db.as_ref()).await.unwrap_err();

    assert!(error.to_string().contains("expected v6 table set"));
    assert!(!relation_exists(db.as_ref(), "seaql_migrations").await);
    assert!(relation_exists(db.as_ref(), "scope_unexpected_production_table").await);
}

#[tokio::test]
async fn invalid_v6_marker_is_refused_without_stamping_migration_state() {
    let (_target, db, _lease) = isolated_database().await;
    initialize_ready_v6(db.as_ref()).await;
    db.execute_unprepared("UPDATE scope_metadata_schema SET version = 5")
        .await
        .unwrap();

    let error = migrations::apply(db.as_ref()).await.unwrap_err();

    assert!(error.to_string().contains("expected current ready v6"));
    assert!(!relation_exists(db.as_ref(), "seaql_migrations").await);
}

#[tokio::test]
async fn partial_schema_is_refused_without_deleting_existing_objects() {
    let (_target, db, _lease) = isolated_database().await;
    db.execute_unprepared(
        "
            CREATE TABLE scope_users (
                id character varying PRIMARY KEY,
                sentinel text NOT NULL
            );
            INSERT INTO scope_users (id, sentinel) VALUES ('legacy', 'keep');
        ",
    )
    .await
    .unwrap();

    let error = migrations::apply(db.as_ref()).await.unwrap_err();

    assert!(error.to_string().contains("expected v6 table set"));
    assert!(!relation_exists(db.as_ref(), "seaql_migrations").await);
    let sentinel = db
        .query_one(Statement::from_string(
            DatabaseBackend::Postgres,
            "SELECT sentinel FROM scope_users WHERE id = 'legacy'".to_string(),
        ))
        .await
        .unwrap()
        .unwrap()
        .try_get::<String>("", "sentinel")
        .unwrap();
    assert_eq!(sentinel, "keep");
}

#[tokio::test]
async fn multiple_v6_markers_are_refused_without_stamping_migration_state() {
    let (_target, db, _lease) = isolated_database().await;
    initialize_ready_v6(db.as_ref()).await;
    db.execute_unprepared(
        "
            INSERT INTO scope_metadata_schema (key, version, deploy_revision, ready)
            VALUES ('unexpected', 6, 'legacy-v6', TRUE)
        ",
    )
    .await
    .unwrap();

    let error = migrations::apply(db.as_ref()).await.unwrap_err();

    assert!(error.to_string().contains("expected one v6 marker"));
    assert!(!relation_exists(db.as_ref(), "seaql_migrations").await);
}

#[tokio::test]
async fn reapplying_latest_migrations_is_a_data_preserving_noop() {
    let (_target, db, _lease) = isolated_database().await;
    migrations::apply(db.as_ref()).await.unwrap();
    db.execute_unprepared(
        "
            INSERT INTO scope_users (id, handle, email, email_verified)
            VALUES ('user_latest', 'latest', 'latest@scope.test', TRUE)
        ",
    )
    .await
    .unwrap();
    let before = representative_business_snapshot(db.as_ref()).await;

    migrations::apply(db.as_ref()).await.unwrap();

    assert_eq!(representative_business_snapshot(db.as_ref()).await, before);
    assert_eq!(
        applied_versions(db.as_ref()).await,
        [
            "m0001_adopt_v6",
            "m0002_retire_reset_schema",
            "m0003_structured_run_attempts",
        ]
    );
}

#[tokio::test]
async fn concurrent_api_migration_attempts_serialize() {
    let (_target, db, _lease) = isolated_database().await;

    let (first, second) = tokio::join!(
        migrations::apply(db.as_ref()),
        migrations::apply(db.as_ref())
    );

    first.unwrap();
    second.unwrap();
    assert_eq!(
        applied_versions(db.as_ref()).await,
        [
            "m0001_adopt_v6",
            "m0002_retire_reset_schema",
            "m0003_structured_run_attempts",
        ]
    );
}

#[tokio::test]
async fn exact_state_check_is_read_only_and_rejects_behind_and_ahead() {
    let (_target, db, _lease) = isolated_database().await;

    assert!(migrations::assert_exact_state(db.as_ref()).await.is_err());
    assert!(!relation_exists(db.as_ref(), "seaql_migrations").await);

    migrations::apply(db.as_ref()).await.unwrap();
    db.execute_unprepared(
        "DELETE FROM seaql_migrations WHERE version = 'm0002_retire_reset_schema'",
    )
    .await
    .unwrap();
    assert!(migrations::assert_exact_state(db.as_ref()).await.is_err());

    db.execute_unprepared(
        "
            INSERT INTO seaql_migrations (version, applied_at)
            VALUES ('m0002_retire_reset_schema', 0), ('m9999_unknown', 0)
        ",
    )
    .await
    .unwrap();
    assert!(migrations::assert_exact_state(db.as_ref()).await.is_err());
}

#[tokio::test]
async fn worker_waits_for_api_migration_and_then_detects_ahead_state() {
    let (target, db, _lease) = isolated_database().await;
    let worker_url = target.schema_database_url();
    let worker_probe = Database::connect(&worker_url).await.unwrap();
    let worker_schema = worker_probe
        .query_one(Statement::from_string(
            DatabaseBackend::Postgres,
            "SELECT current_schema() AS schema".to_string(),
        ))
        .await
        .unwrap()
        .unwrap()
        .try_get::<String>("", "schema")
        .unwrap();
    assert!(worker_schema.starts_with("scope_test_"));
    worker_probe.close().await.unwrap();
    let worker = tokio::spawn(async move {
        connect_postgres_worker_store_with_schema_wait(
            worker_url,
            Duration::from_secs(3),
            Duration::from_millis(20),
        )
        .await
    });
    tokio::time::sleep(Duration::from_millis(75)).await;
    assert!(!worker.is_finished());

    migrations::apply(db.as_ref()).await.unwrap();

    let worker_store = tokio::time::timeout(Duration::from_secs(2), worker)
        .await
        .expect("worker should observe completed API migrations")
        .unwrap()
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
