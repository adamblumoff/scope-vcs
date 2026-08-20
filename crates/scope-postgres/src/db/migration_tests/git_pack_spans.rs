use super::{initialize_ready_v6, isolated_database};
use crate::migrations;
use sea_orm::{ConnectionTrait, DatabaseBackend, Statement};
use sea_orm_migration::MigratorTrait;

#[tokio::test]
async fn existing_segments_become_singleton_pack_spans() {
    let (_target, db, _lease) = isolated_database().await;
    initialize_ready_v6(db.as_ref()).await;
    migrations::Migrator::up(db.as_ref(), Some(21))
        .await
        .unwrap();
    db.execute_unprepared(
        r#"
            INSERT INTO scope_users (id, handle, email, email_verified)
            VALUES ('user_pack_span', 'pack-span', 'pack-span@scope.test', TRUE);
            INSERT INTO scope_repositories (
                id, owner_handle, name, owner_user_id, publication_state,
                change_version, repo_config, policy
            ) VALUES (
                'repo_pack_span', 'pack-span', 'repo', 'user_pack_span', 'Ready',
                2,
                '{"kind":"scope.repo-config","version":1,"visibility":{"default":"private","rules":[]}}'::jsonb,
                '{"default_visibility":"Private","rules":[]}'::jsonb
            );
            INSERT INTO scope_git_heads (
                repo_id, head_oid, segment_sequence, change_version,
                manifest_object_key, manifest_sha256, manifest_size_bytes
            ) VALUES (
                'repo_pack_span', 'head-2', 2, 2,
                '{"GitManifestSha256":"manifest-2"}', 'manifest-2', 10
            );
            INSERT INTO scope_git_segments (
                repo_id, sequence, base_oid, head_oid, object_key, sha256, size_bytes,
                manifest_object_key, manifest_sha256, manifest_size_bytes
            ) VALUES
                (
                    'repo_pack_span', 1, NULL, 'head-1',
                    '{"GitSegmentSha256":"pack-1"}', 'pack-1', 100,
                    '{"GitManifestSha256":"manifest-1"}', 'manifest-1', 10
                ),
                (
                    'repo_pack_span', 2, 'head-1', 'head-2',
                    '{"GitSegmentSha256":"pack-2"}', 'pack-2', 120,
                    '{"GitManifestSha256":"manifest-2"}', 'manifest-2', 10
                );
            INSERT INTO scope_object_references (object_key, ref_kind, ref_id)
            VALUES
                ('{"GitManifestSha256":"manifest-1"}', 'git_segment_manifest', 'repo_pack_span:1'),
                ('{"GitManifestSha256":"manifest-2"}', 'git_segment_manifest', 'repo_pack_span:2'),
                ('{"GitManifestSha256":"manifest-2"}', 'git_manifest', 'repo_pack_span');
        "#,
    )
    .await
    .unwrap();

    migrations::Migrator::up(db.as_ref(), Some(1))
        .await
        .unwrap();

    let spans = db
        .query_all(Statement::from_string(
            DatabaseBackend::Postgres,
            r#"
                SELECT first_sequence, last_sequence, geometric_tier
                FROM scope_git_segments
                WHERE repo_id = 'repo_pack_span'
                ORDER BY first_sequence
            "#
            .to_string(),
        ))
        .await
        .unwrap()
        .into_iter()
        .map(|row| {
            (
                row.try_get::<i64>("", "first_sequence").unwrap(),
                row.try_get::<i64>("", "last_sequence").unwrap(),
                row.try_get::<i32>("", "geometric_tier").unwrap(),
            )
        })
        .collect::<Vec<_>>();
    assert_eq!(spans, [(1, 1, 0), (2, 2, 0)]);

    let schema = db
        .query_one(Statement::from_string(
            DatabaseBackend::Postgres,
            r#"
                SELECT
                    EXISTS (
                        SELECT 1 FROM information_schema.columns
                        WHERE table_schema = current_schema()
                          AND table_name = 'scope_git_heads'
                          AND column_name = 'push_sequence'
                    ) AS has_push_sequence,
                    EXISTS (
                        SELECT 1 FROM information_schema.columns
                        WHERE table_schema = current_schema()
                          AND table_name = 'scope_git_segments'
                          AND column_name = 'manifest_object_key'
                    ) AS has_span_manifest,
                    (
                        SELECT count(*) FROM scope_object_references
                        WHERE ref_kind = 'git_segment_manifest'
                    ) AS stale_manifest_references,
                    (
                        SELECT count(*) FROM scope_orphan_object_jobs
                        WHERE object_key = '{"GitManifestSha256":"manifest-1"}'
                    ) AS retired_manifest_jobs,
                    (
                        SELECT count(*) FROM scope_orphan_object_jobs
                        WHERE object_key = '{"GitManifestSha256":"manifest-2"}'
                    ) AS live_manifest_jobs
            "#
            .to_string(),
        ))
        .await
        .unwrap()
        .unwrap();
    assert!(schema.try_get::<bool>("", "has_push_sequence").unwrap());
    assert!(!schema.try_get::<bool>("", "has_span_manifest").unwrap());
    assert_eq!(
        schema
            .try_get::<i64>("", "stale_manifest_references")
            .unwrap(),
        0
    );
    assert_eq!(
        schema.try_get::<i64>("", "retired_manifest_jobs").unwrap(),
        1
    );
    assert_eq!(schema.try_get::<i64>("", "live_manifest_jobs").unwrap(), 0);
}
