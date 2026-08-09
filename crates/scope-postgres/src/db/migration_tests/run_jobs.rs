use super::super::RunStore;
use super::isolated_database;
use crate::migrations;
use scope_domain::runs::{
    run::{Run, RunTrigger},
    workflow::{
        CompiledWorkflow, RunnerSelector, WorkflowIdentity, WorkflowPath, WorkflowRevision,
    },
};
use sea_orm::{ConnectionTrait, DatabaseBackend, DatabaseConnection, Statement};
use sea_orm_migration::MigratorTrait;

#[tokio::test]
async fn terminal_run_attempts_migrate_as_history_without_active_job_pointers() {
    let (_target, db, _lease) = isolated_database().await;
    migrations::Migrator::up(db.as_ref(), Some(13))
        .await
        .unwrap();
    db.execute_unprepared(
        "INSERT INTO scope_users (id, handle, email, email_verified)
         VALUES ('user_history', 'history', 'history@scope.test', TRUE);
         INSERT INTO scope_repositories (
             id, owner_handle, name, owner_user_id, publication_state,
             change_version, repo_config, policy
         ) VALUES (
             'repo_history', 'history', 'repo', 'user_history', 'Ready',
             1, '{}'::jsonb, '{}'::jsonb
         );
         INSERT INTO scope_runners (
             id, owner_user_id, secret_hash, version, protocol_version,
             capabilities, enabled, created_at_unix, last_seen_at_unix
         ) VALUES (
             'runner_history', 'user_history', repeat('f', 64), '0.1.0', 4,
             '{}'::jsonb, TRUE, 1, 2
         );
         INSERT INTO scope_workflow_revisions (digest, definition, created_at_unix)
         VALUES (
             repeat('a', 64),
             jsonb_build_object(
                 'name', 'History',
                 'triggers', jsonb_build_object('manual', true, 'push_main', false),
                 'jobs', jsonb_build_array(jsonb_build_object(
                     'id', 'checks',
                     'needs', jsonb_build_array(),
                     'runner', jsonb_build_object('kind', 'any'),
                     'container', jsonb_build_object('image', 'rust:1.90'),
                     'timeout_seconds', 1200,
                     'caches', jsonb_build_array(),
                     'steps', jsonb_build_array(jsonb_build_object('name', 'Test', 'run', 'true'))
                 ))
             ),
             1
         );
         INSERT INTO scope_runs (
             id, idempotency_key, repo_id, workflow_path,
             workflow_revision_digest, trigger, requested_by_user_id, source,
             pinned_container_image, desired_runner_name, state,
             cancellation_requested, last_attempt_number, current_attempt_id,
             created_at_unix, updated_at_unix, completed_at_unix
         )
         SELECT 'run_' || state, 'manual:' || state, 'repo_history',
                '/.scope/runs/history.yml', repeat('a', 64), 'manual', 'user_history',
                jsonb_build_object(
                    'kind', 'ephemeral-git-bundle',
                    'object', jsonb_build_object(
                        'sha256', repeat('b', 64),
                        'git_oid', repeat('c', 40)
                    )
                ),
                CASE WHEN state = 'succeeded'
                     THEN 'registry.example/job@sha256:' || repeat('d', 64)
                     ELSE NULL
                END,
                NULL, state, state = 'canceled', 1, NULL, 1, 3, 3
         FROM unnest(ARRAY['succeeded', 'failed', 'canceled', 'lost']) AS state;
         INSERT INTO scope_run_attempts (
             id, run_id, number, runner_id, runner_name, token_hash,
             token_expires_at_unix, state, lease_expires_at_unix,
             last_heartbeat_at_unix, created_at_unix, started_at_unix,
             completed_at_unix, terminal_reason, log_bytes, logs_truncated
         )
         SELECT 'attempt_' || state, 'run_' || state, 1, 'runner_history', 'linux',
                repeat(CASE state
                    WHEN 'succeeded' THEN '1'
                    WHEN 'failed' THEN '2'
                    WHEN 'canceled' THEN '3'
                    ELSE '4'
                END, 64),
                100, state, 100, 2, 1,
                CASE WHEN state = 'succeeded' THEN 2 ELSE NULL END,
                3,
                CASE state
                    WHEN 'succeeded' THEN NULL
                    WHEN 'failed' THEN jsonb_build_object(
                        'kind', 'runner-setup-failed', 'exit_code', 1, 'message', 'failed'
                    )
                    WHEN 'canceled' THEN jsonb_build_object(
                        'kind', 'canceled', 'step_index', NULL
                    )
                    ELSE jsonb_build_object('kind', 'runner-lost', 'step_index', NULL)
                END,
                0, FALSE
         FROM unnest(ARRAY['succeeded', 'failed', 'canceled', 'lost']) AS state;
         UPDATE scope_runs
         SET current_attempt_id = 'attempt_' || state
         WHERE id = 'run_' || state",
    )
    .await
    .unwrap();

    migrations::apply_in_maintenance(db.as_ref()).await.unwrap();

    let summary = db
        .query_one(Statement::from_string(
            DatabaseBackend::Postgres,
            "SELECT count(*) AS job_count,
                    count(*) FILTER (WHERE job.current_attempt_id IS NULL) AS cleared_jobs,
                    count(attempt.id) AS attempt_count,
                    count(*) FILTER (WHERE attempt.job_key = 'checks') AS keyed_attempts
             FROM scope_run_jobs job
             JOIN scope_run_attempts attempt ON attempt.run_id = job.run_id
             WHERE job.run_id LIKE 'run_%'"
                .to_string(),
        ))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(summary.try_get::<i64>("", "job_count").unwrap(), 4);
    assert_eq!(summary.try_get::<i64>("", "cleared_jobs").unwrap(), 4);
    assert_eq!(summary.try_get::<i64>("", "attempt_count").unwrap(), 4);
    assert_eq!(summary.try_get::<i64>("", "keyed_attempts").unwrap(), 4);
}

#[tokio::test]
async fn migrated_manual_runner_targets_remain_idempotent_without_guessing_raw_overrides() {
    let (_target, db, _lease) = isolated_database().await;
    migrations::Migrator::up(db.as_ref(), Some(12))
        .await
        .unwrap();
    db.execute_unprepared(
        "INSERT INTO scope_users (id, handle, email, email_verified)
         VALUES ('user_runner', 'runner', 'runner@scope.test', TRUE);
         INSERT INTO scope_repositories (
             id, owner_handle, name, owner_user_id, publication_state,
             change_version, repo_config, policy
         ) VALUES (
             'repo_runner', 'runner', 'repo', 'user_runner', 'Ready',
             1, '{}'::jsonb, '{}'::jsonb
         );
         INSERT INTO scope_workflow_revisions (digest, definition, created_at_unix)
         VALUES
         (
             repeat('a', 64),
             jsonb_build_object(
                 'name', 'Named',
                 'triggers', jsonb_build_object('manual', true, 'push_main', false),
                 'runner', jsonb_build_object('kind', 'named', 'name', 'linux-workflow'),
                 'container', jsonb_build_object('image', 'rust:1.90'),
                 'timeout_seconds', 1200,
                 'caches', jsonb_build_array(),
                 'steps', jsonb_build_array(jsonb_build_object('name', 'Test', 'run', 'true'))
             ),
             1
         ),
         (
             repeat('b', 64),
             jsonb_build_object(
                 'name', 'Override',
                 'triggers', jsonb_build_object('manual', true, 'push_main', false),
                 'runner', jsonb_build_object('kind', 'any'),
                 'container', jsonb_build_object('image', 'rust:1.90'),
                 'timeout_seconds', 1200,
                 'caches', jsonb_build_array(),
                 'steps', jsonb_build_array(jsonb_build_object('name', 'Test', 'run', 'true'))
             ),
             1
         );
         INSERT INTO scope_runs (
             id, idempotency_key, repo_id, workflow_path,
             workflow_revision_digest, trigger, requested_by_user_id, source,
             pinned_container_image, desired_runner_name, state,
             cancellation_requested, last_attempt_number, current_attempt_id,
             created_at_unix, updated_at_unix, completed_at_unix
         ) VALUES
         (
             'run_named', 'manual:named', 'repo_runner', '/.scope/runs/named.yml',
             repeat('a', 64), 'manual', 'user_runner',
             jsonb_build_object(
                 'kind', 'ephemeral-git-bundle',
                 'object', jsonb_build_object(
                     'content_ref', jsonb_build_object('GitBundleSha256', repeat('c', 64)),
                     'sha256', repeat('c', 64),
                     'git_oid', repeat('d', 40),
                     'git_file_mode', '100644',
                     'size_bytes', 1
                 )
             ),
             NULL, 'linux-workflow', 'queued', FALSE, 0, NULL, 1, 1, NULL
         ),
         (
             'run_override', 'manual:override', 'repo_runner', '/.scope/runs/override.yml',
             repeat('b', 64), 'manual', 'user_runner',
             jsonb_build_object(
                 'kind', 'ephemeral-git-bundle',
                 'object', jsonb_build_object(
                     'content_ref', jsonb_build_object('GitBundleSha256', repeat('e', 64)),
                     'sha256', repeat('e', 64),
                     'git_oid', repeat('f', 40),
                     'git_file_mode', '100644',
                     'size_bytes', 1
                 )
             ),
             NULL, 'linux-override', 'queued', FALSE, 0, NULL, 1, 1, NULL
         )",
    )
    .await
    .unwrap();

    migrations::apply_in_maintenance(db.as_ref()).await.unwrap();
    db.execute_unprepared(
        "UPDATE scope_runner_protocol_cutover
         SET state = 'v7-open'
         WHERE key = 'current'",
    )
    .await
    .unwrap();

    let runs = RunStore { db: db.clone() };
    let named_revision = migrated_revision(db.as_ref(), "run_named").await;
    let named = runs.run("run_named").await.unwrap().unwrap();
    assert!(named.runner_override.is_none());
    let named_retry = Run::new(
        "retry_named",
        &named.idempotency_key,
        named_revision.workflow().clone(),
        named_revision.digest(),
        RunTrigger::Manual,
        named.requested_by_user_id.clone(),
        named.source.clone(),
        None,
        1,
    )
    .unwrap();
    assert_eq!(
        runs.enqueue_run(named_retry, named_revision)
            .await
            .unwrap()
            .id,
        "run_named"
    );

    let override_revision = migrated_revision(db.as_ref(), "run_override").await;
    let overridden = runs.run("run_override").await.unwrap().unwrap();
    assert_eq!(
        overridden.runner_override,
        Some(RunnerSelector::named("linux-override").unwrap())
    );
    let override_retry = Run::new(
        "retry_override",
        &overridden.idempotency_key,
        override_revision.workflow().clone(),
        override_revision.digest(),
        RunTrigger::Manual,
        overridden.requested_by_user_id.clone(),
        overridden.source.clone(),
        Some(RunnerSelector::named("linux-override").unwrap()),
        1,
    )
    .unwrap();
    assert_eq!(
        runs.enqueue_run(override_retry, override_revision)
            .await
            .unwrap()
            .id,
        "run_override"
    );
}

async fn migrated_revision(db: &DatabaseConnection, run_id: &str) -> WorkflowRevision {
    let row = db
        .query_one(Statement::from_sql_and_values(
            DatabaseBackend::Postgres,
            "SELECT run.repo_id, run.workflow_path, revision.definition
             FROM scope_runs run
             JOIN scope_workflow_revisions revision
               ON revision.digest = run.workflow_revision_digest
             WHERE run.id = $1",
            [run_id.into()],
        ))
        .await
        .unwrap()
        .unwrap();
    WorkflowRevision::new(
        WorkflowIdentity::new(
            row.try_get::<String>("", "repo_id").unwrap(),
            WorkflowPath::parse(row.try_get::<String>("", "workflow_path").unwrap()).unwrap(),
        )
        .unwrap(),
        serde_json::from_value::<CompiledWorkflow>(
            row.try_get::<serde_json::Value>("", "definition").unwrap(),
        )
        .unwrap(),
    )
    .unwrap()
}

#[tokio::test]
async fn workflow_runtime_migrations_reset_protocol_authority_to_v7_fenced() {
    let (_target, db, _lease) = isolated_database().await;
    migrations::Migrator::up(db.as_ref(), Some(13))
        .await
        .unwrap();
    db.execute_unprepared(
        "INSERT INTO scope_users (id, handle, email, email_verified)
         VALUES ('user_canary', 'canary', 'canary@scope.test', TRUE);
         INSERT INTO scope_repositories (
             id, owner_handle, name, owner_user_id, publication_state,
             change_version, repo_config, policy
         ) VALUES (
             'repo_canary', 'canary', 'repo', 'user_canary', 'Published',
             1, '{}'::jsonb, '{}'::jsonb
         );
         INSERT INTO scope_runners (
             id, owner_user_id, secret_hash, version, protocol_version,
             capabilities, enabled, created_at_unix, last_seen_at_unix
         ) VALUES (
             'runner_canary', 'user_canary', repeat('f', 64), '0.1.0', 4,
             '{}'::jsonb, TRUE, 1, 2
         );
         INSERT INTO scope_workflow_revisions (digest, definition, created_at_unix)
         VALUES (
             repeat('a', 64),
             jsonb_build_object(
                 'name', 'Canary',
                 'triggers', jsonb_build_object('manual', true, 'push_main', false),
                 'jobs', jsonb_build_array(jsonb_build_object(
                     'id', 'checks',
                     'needs', jsonb_build_array(),
                     'runner', jsonb_build_object('kind', 'named', 'name', 'remote-linux'),
                     'container', jsonb_build_object('image', 'alpine:3.20'),
                     'timeout_seconds', 300,
                     'caches', jsonb_build_array(),
                     'steps', jsonb_build_array(jsonb_build_object('name', 'Test', 'run', 'true'))
                 ))
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
             'run_canary', 'manual:canary', 'repo_canary', '/.scope/runs/canary.yml',
             repeat('a', 64), 'manual', 'user_canary',
             jsonb_build_object(
                 'kind', 'ephemeral-git-bundle',
                 'object', jsonb_build_object(
                     'sha256', repeat('b', 64),
                     'git_oid', repeat('c', 40)
                 )
             ),
             NULL, 'remote-linux', 'queued', FALSE, 0, NULL, 1, 1, NULL
         );
         UPDATE scope_runner_protocol_cutover
         SET state = 'v4-open', canary_generation = 7
         WHERE key = 'current';
         INSERT INTO scope_runner_protocol_canaries (
             generation, phase, runner_id, run_id, status,
             created_at_unix, updated_at_unix
         ) VALUES (
             7, 'cold-write', 'runner_canary', 'run_canary', 'pending', 2, 2
         )",
    )
    .await
    .unwrap();

    migrations::Migrator::up(db.as_ref(), Some(1))
        .await
        .unwrap();
    db.execute_unprepared(
        "UPDATE scope_runs
         SET state = 'queued', cancellation_requested = FALSE,
             updated_at_unix = 3, completed_at_unix = NULL
         WHERE id = 'run_canary';
         UPDATE scope_run_jobs
         SET state = 'queued', updated_at_unix = 3, completed_at_unix = NULL
         WHERE run_id = 'run_canary';
         UPDATE scope_runner_protocol_cutover
         SET state = 'v5-fenced', canary_generation = 7
         WHERE key = 'current';
         INSERT INTO scope_runner_protocol_canaries (
             generation, phase, runner_id, run_id, status,
             created_at_unix, updated_at_unix
         ) VALUES (
             7, 'cold-write', 'runner_canary', 'run_canary', 'pending', 3, 3
         )",
    )
    .await
    .unwrap();
    migrations::Migrator::up(db.as_ref(), None).await.unwrap();

    let cutover = db
        .query_one(Statement::from_string(
            DatabaseBackend::Postgres,
            "SELECT state, canary_generation
             FROM scope_runner_protocol_cutover
             WHERE key = 'current'"
                .to_string(),
        ))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(cutover.try_get::<String>("", "state").unwrap(), "v7-fenced");
    assert_eq!(cutover.try_get::<i64>("", "canary_generation").unwrap(), 0);
    let retired = db
        .query_one(Statement::from_string(
            DatabaseBackend::Postgres,
            "SELECT run.state AS run_state, run.cancellation_requested,
                    run.completed_at_unix AS run_completed_at_unix,
                    job.state AS job_state,
                    job.completed_at_unix AS job_completed_at_unix
             FROM scope_runs run
             JOIN scope_run_jobs job ON job.run_id = run.id
             WHERE run.id = 'run_canary'"
                .to_string(),
        ))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        retired.try_get::<String>("", "run_state").unwrap(),
        "canceled"
    );
    assert!(
        retired
            .try_get::<bool>("", "cancellation_requested")
            .unwrap()
    );
    assert_eq!(
        retired.try_get::<String>("", "job_state").unwrap(),
        "canceled"
    );
    assert_eq!(
        retired.try_get::<i64>("", "run_completed_at_unix").unwrap(),
        retired.try_get::<i64>("", "job_completed_at_unix").unwrap()
    );
    let canary_count = db
        .query_one(Statement::from_string(
            DatabaseBackend::Postgres,
            "SELECT count(*) AS count FROM scope_runner_protocol_canaries".to_string(),
        ))
        .await
        .unwrap()
        .unwrap()
        .try_get::<i64>("", "count")
        .unwrap();
    assert_eq!(canary_count, 0);
    assert!(
        db.execute_unprepared(
            "UPDATE scope_runner_protocol_cutover SET state = 'v4-open' WHERE key = 'current'"
        )
        .await
        .is_err()
    );
}
