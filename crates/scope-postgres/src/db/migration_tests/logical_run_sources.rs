use super::{initialize_ready_v6, isolated_database};
use crate::migrations;
use sea_orm::{ConnectionTrait, DatabaseBackend, Statement};
use sea_orm_migration::MigratorTrait;

#[tokio::test]
async fn accepted_revision_runs_keep_their_exact_bundle_after_cutover() {
    let (_target, db, _lease) = isolated_database().await;
    initialize_ready_v6(db.as_ref()).await;
    migrations::Migrator::up(db.as_ref(), Some(22))
        .await
        .unwrap();
    db.execute_unprepared(
        r#"
            INSERT INTO scope_users (id, handle, email, email_verified)
            VALUES ('legacy_run_user', 'legacy-run', 'legacy-run@scope.test', TRUE);
            INSERT INTO scope_repositories (
                id, owner_handle, name, owner_user_id, publication_state,
                change_version, repo_config, policy
            ) VALUES (
                'legacy_run_repo', 'legacy-run', 'repo', 'legacy_run_user', 'Ready',
                1,
                '{"kind":"scope.repo-config","version":1,"visibility":{"default":"private","rules":[]}}'::jsonb,
                '{"default_visibility":"Private","rules":[]}'::jsonb
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
                    'legacy_retired', 'legacy-retired', 'legacy_run_repo', '.scope/runs/test.yaml',
                    repeat('1', 64), 'push-main', NULL,
                    jsonb_build_object(
                        'kind', 'accepted-revision',
                        'change_version', 1,
                        'manifest', jsonb_build_object(
                            'content_ref', jsonb_build_object('GitManifestSha256', repeat('a', 64)),
                            'sha256', repeat('a', 64), 'git_oid', repeat('c', 40),
                            'git_file_mode', '100644', 'size_bytes', 10
                        ),
                        'snapshot', jsonb_build_object(
                            'content_ref', jsonb_build_object('GitBundleSha256', repeat('b', 64)),
                            'sha256', repeat('b', 64), 'git_oid', repeat('c', 40),
                            'git_file_mode', '100644', 'size_bytes', 100
                        ),
                        'audience', 'Public'
                    ),
                    'succeeded', FALSE, 1, 2, 2
                ),
                (
                    'legacy_shared', 'legacy-shared', 'legacy_run_repo', '.scope/runs/test.yaml',
                    repeat('1', 64), 'push-main', NULL,
                    jsonb_build_object(
                        'kind', 'accepted-revision',
                        'change_version', 1,
                        'manifest', jsonb_build_object(
                            'content_ref', jsonb_build_object('GitManifestSha256', repeat('d', 64)),
                            'sha256', repeat('d', 64), 'git_oid', repeat('f', 40),
                            'git_file_mode', '100644', 'size_bytes', 20
                        ),
                        'snapshot', jsonb_build_object(
                            'content_ref', jsonb_build_object('GitBundleSha256', repeat('e', 64)),
                            'sha256', repeat('e', 64), 'git_oid', repeat('f', 40),
                            'git_file_mode', '100644', 'size_bytes', 200
                        ),
                        'audience', 'Private'
                    ),
                    'succeeded', FALSE, 1, 2, 2
                );
            INSERT INTO scope_object_references (object_key, ref_kind, ref_id)
            VALUES
                (jsonb_build_object('GitManifestSha256', repeat('a', 64))::text, 'run_source', 'legacy_retired'),
                (jsonb_build_object('GitBundleSha256', repeat('b', 64))::text, 'run_source', 'legacy_retired'),
                (jsonb_build_object('GitManifestSha256', repeat('d', 64))::text, 'run_source', 'legacy_shared'),
                (jsonb_build_object('GitBundleSha256', repeat('e', 64))::text, 'run_source', 'legacy_shared'),
                (jsonb_build_object('GitManifestSha256', repeat('d', 64))::text, 'git_manifest', 'legacy_run_repo');
            INSERT INTO scope_push_trigger_evaluations (
                repo_id, change_version, head_oid, state, message, checks,
                created_at_unix, completed_at_unix
            ) VALUES (
                'legacy_run_repo', 2, repeat('7', 40), 'pending', NULL, '[]'::jsonb, 1, NULL
            );
            INSERT INTO scope_outbox_jobs (
                id, idempotency_key, kind, repo_id, repo_version, payload,
                state, attempts, next_run_at_unix, lease_owner, lease_expires_at_unix,
                last_error, created_at_unix, updated_at_unix, completed_at_unix
            ) VALUES (
                'legacy_push_job', 'push_main_trigger_evaluation:legacy_run_repo:2',
                'push_main_trigger_evaluation', 'legacy_run_repo', 2,
                jsonb_build_object(
                    'workflow_schema_version', 4,
                    'manifest', jsonb_build_object(
                        'content_ref', jsonb_build_object('GitManifestSha256', repeat('8', 64)),
                        'sha256', repeat('8', 64), 'git_oid', repeat('7', 40),
                        'git_file_mode', '100644', 'size_bytes', 30
                    ),
                    'input', jsonb_build_object(
                        'head_oid', repeat('7', 40),
                        'snapshot', jsonb_build_object(
                            'content_ref', jsonb_build_object('GitBundleSha256', repeat('9', 64)),
                            'sha256', repeat('9', 64), 'git_oid', repeat('7', 40),
                            'git_file_mode', '100644', 'size_bytes', 300
                        ),
                        'workflows', '[]'::jsonb,
                        'configuration_error', NULL
                    )
                ),
                'ready', 0, 1, NULL, NULL, NULL, 1, 1, NULL
            );
            INSERT INTO scope_object_references (object_key, ref_kind, ref_id)
            VALUES
                (jsonb_build_object('GitManifestSha256', repeat('8', 64))::text,
                 'push_trigger_source', 'legacy_run_repo:2'),
                (jsonb_build_object('GitBundleSha256', repeat('9', 64))::text,
                 'push_trigger_source', 'legacy_run_repo:2');
        "#,
    )
    .await
    .unwrap();

    migrations::Migrator::up(db.as_ref(), Some(1))
        .await
        .unwrap();

    let state = db
        .query_one(Statement::from_string(
            DatabaseBackend::Postgres,
            r#"
                SELECT
                    count(*) FILTER (
                        WHERE source->>'kind' = 'ephemeral-git-bundle'
                    ) AS converted_runs,
                    count(*) FILTER (
                        WHERE source->>'kind' = 'accepted-revision'
                    ) AS legacy_runs,
                    count(*) FILTER (
                        WHERE id = 'legacy_retired'
                          AND source#>>'{object,sha256}' = repeat('b', 64)
                    ) AS exact_retired_bundle,
                    count(*) FILTER (
                        WHERE id = 'legacy_shared'
                          AND source#>>'{object,sha256}' = repeat('e', 64)
                    ) AS exact_shared_bundle
                FROM scope_runs
            "#
            .to_string(),
        ))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(state.try_get::<i64>("", "converted_runs").unwrap(), 2);
    assert_eq!(state.try_get::<i64>("", "legacy_runs").unwrap(), 0);
    assert_eq!(state.try_get::<i64>("", "exact_retired_bundle").unwrap(), 1);
    assert_eq!(state.try_get::<i64>("", "exact_shared_bundle").unwrap(), 1);

    let references = db
        .query_one(Statement::from_string(
            DatabaseBackend::Postgres,
            r#"
                SELECT
                    count(*) FILTER (
                        WHERE ref_kind = 'run_source'
                          AND object_key::jsonb ? 'GitManifestSha256'
                    ) AS stale_manifest_refs,
                    count(*) FILTER (
                        WHERE ref_kind = 'run_source'
                          AND object_key::jsonb ? 'GitBundleSha256'
                    ) AS bundle_refs
                FROM scope_object_references
            "#
            .to_string(),
        ))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        references
            .try_get::<i64>("", "stale_manifest_refs")
            .unwrap(),
        0
    );
    assert_eq!(references.try_get::<i64>("", "bundle_refs").unwrap(), 2);

    let retired_push = db
        .query_one(Statement::from_string(
            DatabaseBackend::Postgres,
            r#"
                SELECT
                    (SELECT count(*) FROM scope_outbox_jobs
                     WHERE id = 'legacy_push_job') AS legacy_jobs,
                    (SELECT count(*) FROM scope_object_references
                     WHERE ref_kind = 'push_trigger_source'
                       AND ref_id = 'legacy_run_repo:2') AS legacy_refs,
                    (SELECT count(*) FROM scope_push_trigger_evaluations
                     WHERE repo_id = 'legacy_run_repo' AND change_version = 2
                       AND state = 'failed' AND completed_at_unix IS NOT NULL
                       AND message LIKE 'retired by%') AS failed_evaluations
            "#
            .to_string(),
        ))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(retired_push.try_get::<i64>("", "legacy_jobs").unwrap(), 0);
    assert_eq!(retired_push.try_get::<i64>("", "legacy_refs").unwrap(), 0);
    assert_eq!(
        retired_push
            .try_get::<i64>("", "failed_evaluations")
            .unwrap(),
        1
    );

    let cleanup = db
        .query_one(Statement::from_string(
            DatabaseBackend::Postgres,
            r#"
                SELECT
                    count(*) FILTER (
                        WHERE object_key::jsonb = jsonb_build_object(
                            'GitManifestSha256', repeat('a', 64)
                        )
                    ) AS retired_manifest_jobs,
                    count(*) FILTER (
                        WHERE object_key::jsonb = jsonb_build_object(
                            'GitManifestSha256', repeat('d', 64)
                        )
                    ) AS shared_manifest_jobs,
                    count(*) FILTER (
                        WHERE object_key::jsonb = jsonb_build_object(
                            'GitManifestSha256', repeat('8', 64)
                        ) OR object_key::jsonb = jsonb_build_object(
                            'GitBundleSha256', repeat('9', 64)
                        )
                    ) AS retired_push_source_jobs
                FROM scope_orphan_object_jobs
            "#
            .to_string(),
        ))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        cleanup.try_get::<i64>("", "retired_manifest_jobs").unwrap(),
        1
    );
    assert_eq!(
        cleanup.try_get::<i64>("", "shared_manifest_jobs").unwrap(),
        0
    );
    assert_eq!(
        cleanup
            .try_get::<i64>("", "retired_push_source_jobs")
            .unwrap(),
        2
    );
}
