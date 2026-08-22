use super::{isolated_database, relation_exists};
use crate::migrations;
use sea_orm::{ConnectionTrait, DatabaseBackend, Statement};
use sea_orm_migration::MigratorTrait;

#[tokio::test]
async fn run_creation_sequence_backfills_history_order_and_advances_for_new_runs() {
    let (_target, db, _lease) = isolated_database().await;
    migrations::Migrator::up(db.as_ref(), Some(26))
        .await
        .unwrap();
    db.execute_unprepared(
        r#"
        INSERT INTO scope_users (id, handle, email, email_verified)
        VALUES ('run-order-owner', 'run-order-owner', 'runs@scope.test', TRUE);
        INSERT INTO scope_repositories (
            id, owner_handle, name, owner_user_id, publication_state,
            change_version, repo_config, policy
        ) VALUES (
            'run-order-owner/repo', 'run-order-owner', 'repo', 'run-order-owner', 'Ready',
            1, '{}'::jsonb, '{}'::jsonb
        );
        INSERT INTO scope_workflow_revisions (digest, definition, created_at_unix)
        VALUES (repeat('1', 64), '{"jobs":[{}]}'::jsonb, 1);
        INSERT INTO scope_runs (
            id, idempotency_key, repo_id, workflow_path,
            workflow_revision_digest, trigger, requested_by_user_id,
            source, state, cancellation_requested,
            created_at_unix, updated_at_unix, completed_at_unix
        ) VALUES
            (
                'run_same_second_z', 'z', 'run-order-owner/repo', '.scope/runs/test.yml',
                repeat('1', 64), 'manual', 'run-order-owner',
                '{"kind":"ephemeral-git-bundle","object":{"sha256":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","git_oid":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"}}'::jsonb,
                'queued', FALSE, 10, 10, NULL
            ),
            (
                'run_same_second_a', 'a', 'run-order-owner/repo', '.scope/runs/test.yml',
                repeat('1', 64), 'manual', 'run-order-owner',
                '{"kind":"ephemeral-git-bundle","object":{"sha256":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","git_oid":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"}}'::jsonb,
                'queued', FALSE, 10, 10, NULL
            ),
            (
                'run_later', 'later', 'run-order-owner/repo', '.scope/runs/test.yml',
                repeat('1', 64), 'manual', 'run-order-owner',
                '{"kind":"ephemeral-git-bundle","object":{"sha256":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","git_oid":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"}}'::jsonb,
                'queued', FALSE, 11, 11, NULL
            );
        "#,
    )
    .await
    .unwrap();

    migrations::Migrator::up(db.as_ref(), Some(1))
        .await
        .unwrap();

    let ordered = db
        .query_all(Statement::from_string(
            DatabaseBackend::Postgres,
            "SELECT id, creation_sequence FROM scope_runs ORDER BY creation_sequence".to_string(),
        ))
        .await
        .unwrap();
    assert_eq!(
        ordered
            .iter()
            .map(|row| row.try_get::<String>("", "id").unwrap())
            .collect::<Vec<_>>(),
        ["run_same_second_a", "run_same_second_z", "run_later"]
    );

    db.execute_unprepared(
        r#"
        INSERT INTO scope_runs (
            id, idempotency_key, repo_id, workflow_path,
            workflow_revision_digest, trigger, requested_by_user_id,
            source, state, cancellation_requested,
            created_at_unix, updated_at_unix, completed_at_unix
        ) VALUES (
            'run_new', 'new', 'run-order-owner/repo', '.scope/runs/test.yml',
            repeat('1', 64), 'manual', 'run-order-owner',
            '{"kind":"ephemeral-git-bundle","object":{"sha256":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","git_oid":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"}}'::jsonb,
            'queued', FALSE, 11, 11, NULL
        );
        "#,
    )
    .await
    .unwrap();
    let new_sequence = db
        .query_one(Statement::from_string(
            DatabaseBackend::Postgres,
            "SELECT creation_sequence FROM scope_runs WHERE id = 'run_new'".to_string(),
        ))
        .await
        .unwrap()
        .unwrap()
        .try_get::<i64>("", "creation_sequence")
        .unwrap();
    assert_eq!(new_sequence, 4);
    assert!(relation_exists(db.as_ref(), "idx_scope_runs_history").await);
    assert!(relation_exists(db.as_ref(), "idx_scope_runs_workflow_history").await);
}
