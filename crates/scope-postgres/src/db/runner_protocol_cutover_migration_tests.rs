use super::migration_tests::{applied_versions, isolated_database, relation_exists};
use crate::migrations;
use sea_orm::{ConnectionTrait, DatabaseBackend, Statement};
use sea_orm_migration::MigratorTrait;

#[tokio::test]
async fn v4_cutover_refuses_to_start_until_v3_attempts_are_drained() {
    let (_target, db, _lease) = isolated_database().await;
    migrations::Migrator::up(db.as_ref(), Some(3))
        .await
        .unwrap();
    db.execute_unprepared(
        "
            INSERT INTO scope_users (id, handle, email, email_verified)
            VALUES ('user_cutover', 'cutover', 'cutover@scope.test', TRUE);
            INSERT INTO scope_repositories (
                id, owner_handle, name, owner_user_id, publication_state,
                default_visibility, change_version, repo_config, policy
            ) VALUES (
                'repo_cutover', 'cutover', 'repo', 'user_cutover', 'Published',
                'Private', 1, '{}'::jsonb, '{}'::jsonb
            );
            INSERT INTO scope_runners (
                id, owner_user_id, secret_hash, version, protocol_version,
                capabilities, enabled, created_at_unix, last_seen_at_unix
            ) VALUES (
                'runner_cutover', 'user_cutover', repeat('a', 64), '0.1.0', 3,
                '{}'::jsonb, TRUE, 1, 2
            );
            INSERT INTO scope_workflow_revisions (digest, definition, created_at_unix)
            VALUES (
                repeat('b', 64),
                jsonb_build_object(
                    'name', 'Legacy',
                    'triggers', jsonb_build_object('manual', true, 'push_main', false),
                    'runner', jsonb_build_object('kind', 'any'),
                    'container', jsonb_build_object('image', 'rust:1.90'),
                    'timeout_seconds', 1200,
                    'steps', jsonb_build_array(jsonb_build_object('name', 'Test', 'run', 'cargo test'))
                ),
                1
            );
            INSERT INTO scope_runs (
                id, idempotency_key, repo_id, workflow_path,
                workflow_revision_digest, trigger, requested_by_user_id, source,
                pinned_container_image, desired_runner_name, state,
                cancellation_requested, last_attempt_number, current_attempt_id,
                created_at_unix, updated_at_unix, completed_at_unix
            ) VALUES (
                'run_cutover', 'manual:cutover', 'repo_cutover', '/.scope/runs/test.yml',
                repeat('b', 64), 'manual', 'user_cutover',
                jsonb_build_object(
                    'kind', 'ephemeral-git-bundle',
                    'object', jsonb_build_object(
                        'sha256', repeat('c', 64),
                        'git_oid', repeat('d', 40)
                    )
                ),
                NULL, NULL, 'queued', FALSE, 0, NULL, 1, 1, NULL
            );
            INSERT INTO scope_run_attempts (
                id, run_id, number, runner_id, runner_name, token_hash,
                token_expires_at_unix, state, lease_expires_at_unix,
                last_heartbeat_at_unix, created_at_unix, started_at_unix,
                completed_at_unix, terminal_reason, log_bytes, logs_truncated
            ) VALUES (
                'attempt_cutover', 'run_cutover', 1, 'runner_cutover', 'linux',
                repeat('e', 64), 100, 'leased', 100, 2, 1, NULL,
                NULL, NULL, 0, FALSE
            );
            UPDATE scope_runs
            SET state = 'leased', last_attempt_number = 1,
                current_attempt_id = 'attempt_cutover', updated_at_unix = 2
            WHERE id = 'run_cutover';
        ",
    )
    .await
    .unwrap();

    let error = migrations::apply(db.as_ref()).await.unwrap_err();
    assert!(
        error
            .to_string()
            .contains("requires all V3 attempts to drain")
    );
    assert_eq!(
        applied_versions(db.as_ref()).await,
        [
            "m0001_adopt_v6",
            "m0002_retire_reset_schema",
            "m0003_structured_run_attempts",
        ]
    );
    assert!(!relation_exists(db.as_ref(), "scope_runner_protocol_cutover").await);
    let definition = db
        .query_one(Statement::from_string(
            DatabaseBackend::Postgres,
            "SELECT definition FROM scope_workflow_revisions".to_string(),
        ))
        .await
        .unwrap()
        .unwrap()
        .try_get::<serde_json::Value>("", "definition")
        .unwrap();
    assert!(definition.get("caches").is_none());

    db.execute_unprepared(
        "
            UPDATE scope_run_attempts
            SET state = 'failed', completed_at_unix = 3,
                terminal_reason = jsonb_build_object(
                    'kind', 'setup-failed', 'exit_code', 1, 'message', 'drained'
                );
            UPDATE scope_runs
            SET state = 'failed', current_attempt_id = NULL,
                completed_at_unix = 3, updated_at_unix = 3;
        ",
    )
    .await
    .unwrap();
    migrations::apply(db.as_ref()).await.unwrap();

    let runner_enabled = db
        .query_one(Statement::from_string(
            DatabaseBackend::Postgres,
            "SELECT enabled FROM scope_runners WHERE id = 'runner_cutover'".to_string(),
        ))
        .await
        .unwrap()
        .unwrap()
        .try_get::<bool>("", "enabled")
        .unwrap();
    assert!(!runner_enabled);
    assert!(
        db.execute_unprepared(
            "UPDATE scope_runners SET enabled = TRUE WHERE id = 'runner_cutover'"
        )
        .await
        .is_err()
    );
    assert!(
        db.execute_unprepared(
            "INSERT INTO scope_workflow_revisions (digest, definition, created_at_unix)
             VALUES (repeat('f', 64), '{}'::jsonb, 4)"
        )
        .await
        .is_err()
    );
}

#[tokio::test]
async fn workflow_jobs_rewrite_refuses_to_start_until_attempts_are_drained() {
    let (_target, db, _lease) = isolated_database().await;
    migrations::Migrator::up(db.as_ref(), Some(12))
        .await
        .unwrap();
    db.execute_unprepared(
        "
            INSERT INTO scope_users (id, handle, email, email_verified)
            VALUES ('user_jobs', 'jobs', 'jobs@scope.test', TRUE);
            INSERT INTO scope_repositories (
                id, owner_handle, name, owner_user_id, publication_state,
                change_version, repo_config, policy
            ) VALUES (
                'repo_jobs', 'jobs', 'repo', 'user_jobs', 'Ready',
                1, '{}'::jsonb, '{}'::jsonb
            );
            INSERT INTO scope_runners (
                id, owner_user_id, secret_hash, version, protocol_version,
                capabilities, enabled, created_at_unix, last_seen_at_unix
            ) VALUES (
                'runner_jobs', 'user_jobs', repeat('a', 64), '0.1.0', 4,
                '{\"log_transport\":\"stable-chunks\",\"execution_mode\":\"container-per-job\",\"platform\":\"linux-amd64\"}'::jsonb,
                TRUE, 1, 2
            );
            INSERT INTO scope_workflow_revisions (digest, definition, created_at_unix)
            VALUES (
                repeat('b', 64),
                jsonb_build_object(
                    'name', 'Legacy',
                    'triggers', jsonb_build_object('manual', true, 'push_main', false),
                    'runner', jsonb_build_object('kind', 'any'),
                    'container', jsonb_build_object('image', 'rust:1.90'),
                    'timeout_seconds', 1200,
                    'caches', jsonb_build_array(),
                    'steps', jsonb_build_array(jsonb_build_object('name', 'Test', 'run', 'cargo test'))
                ),
                1
            );
            INSERT INTO scope_runs (
                id, idempotency_key, repo_id, workflow_path,
                workflow_revision_digest, trigger, requested_by_user_id, source,
                pinned_container_image, desired_runner_name, state,
                cancellation_requested, last_attempt_number, current_attempt_id,
                created_at_unix, updated_at_unix, completed_at_unix
            ) VALUES (
                'run_jobs', 'manual:jobs', 'repo_jobs', '/.scope/runs/test.yml',
                repeat('b', 64), 'manual', 'user_jobs',
                jsonb_build_object(
                    'kind', 'ephemeral-git-bundle',
                    'object', jsonb_build_object(
                        'content_ref', concat('git-bundle-sha256:', repeat('c', 64)),
                        'sha256', repeat('c', 64),
                        'git_oid', repeat('d', 40),
                        'git_file_mode', '100644',
                        'size_bytes', 1
                    )
                ),
                NULL, NULL, 'queued', FALSE, 0, NULL, 1, 1, NULL
            );
            INSERT INTO scope_run_attempts (
                id, run_id, number, runner_id, runner_name, token_hash,
                token_expires_at_unix, state, lease_expires_at_unix,
                last_heartbeat_at_unix, created_at_unix, started_at_unix,
                completed_at_unix, terminal_reason, log_bytes, logs_truncated
            ) VALUES (
                'attempt_jobs', 'run_jobs', 1, 'runner_jobs', 'linux',
                repeat('e', 64), 100, 'leased', 100, 2, 1, NULL,
                NULL, NULL, 0, FALSE
            );
            UPDATE scope_runs
            SET state = 'leased', last_attempt_number = 1,
                current_attempt_id = 'attempt_jobs', updated_at_unix = 2
            WHERE id = 'run_jobs';
        ",
    )
    .await
    .unwrap();

    let error = migrations::apply(db.as_ref()).await.unwrap_err();
    assert!(
        error
            .to_string()
            .contains("workflow jobs migration requires all run attempts to drain")
    );
    assert_eq!(
        applied_versions(db.as_ref())
            .await
            .last()
            .map(String::as_str),
        Some("m0012_request_revisions")
    );

    db.execute_unprepared(
        "
            UPDATE scope_run_attempts
            SET state = 'failed', completed_at_unix = 3,
                terminal_reason = jsonb_build_object(
                    'kind', 'setup-failed', 'message', 'drained'
                );
            UPDATE scope_runs
            SET state = 'failed', current_attempt_id = NULL,
                completed_at_unix = 3, updated_at_unix = 3;
        ",
    )
    .await
    .unwrap();
    migrations::apply(db.as_ref()).await.unwrap();

    let row = db
        .query_one(Statement::from_string(
            DatabaseBackend::Postgres,
            "SELECT revision.definition, run.workflow_revision_digest
             FROM scope_runs run
             JOIN scope_workflow_revisions revision
               ON revision.digest = run.workflow_revision_digest
             WHERE run.id = 'run_jobs'"
                .to_string(),
        ))
        .await
        .unwrap()
        .unwrap();
    let definition = row.try_get::<serde_json::Value>("", "definition").unwrap();
    assert_eq!(definition["jobs"][0]["id"], "checks");
    assert!(definition.get("steps").is_none());
    assert_ne!(
        row.try_get::<String>("", "workflow_revision_digest")
            .unwrap(),
        "b".repeat(64)
    );
}
