use super::isolated_database;
use crate::migrations;
use sea_orm::{ConnectionTrait, DatabaseBackend, Statement};
use sea_orm_migration::MigratorTrait;

#[tokio::test]
async fn run_job_migration_resets_the_protocol_authority_to_v5_fenced() {
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
             jsonb_build_object('jobs', jsonb_build_array(jsonb_build_object('id', 'checks'))),
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

    migrations::Migrator::up(db.as_ref(), Some(14))
        .await
        .unwrap();

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
    assert_eq!(cutover.try_get::<String>("", "state").unwrap(), "v5-fenced");
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
