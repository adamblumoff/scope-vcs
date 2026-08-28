use super::isolated_database;
use crate::{db::git_segment_v2_backfill::CREATE_BACKFILL_TABLE, migrations};
use scope_domain::runs::source::RunSource;
use sea_orm::{ConnectionTrait, DatabaseBackend, Statement};
use sea_orm_migration::MigratorTrait;

const REPOSITORY_ID: &str = "cutover-user/repo";
const LEGACY_SHA256: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const SEGMENT_ID: &str = "11111111111111111111111111111111";

#[tokio::test]
async fn v2_cutover_refuses_segments_without_completed_object_backfill() {
    let (_target, db, _lease) = isolated_database().await;
    migrations::Migrator::up(db.as_ref(), Some(32))
        .await
        .unwrap();
    insert_legacy_git_state(db.as_ref()).await;
    db.execute_unprepared(CREATE_BACKFILL_TABLE).await.unwrap();

    let error = migrations::Migrator::up(db.as_ref(), Some(1))
        .await
        .unwrap_err();
    assert!(
        error
            .to_string()
            .contains("requires every legacy Git segment to be backfilled")
    );
    assert!(!table_exists(db.as_ref(), "scope_git_segment_uploads").await);
    assert!(column_exists(db.as_ref(), "scope_git_segments", "object_key").await);
}

#[tokio::test]
async fn v2_cutover_preserves_segments_and_pinned_sources_from_completed_backfill() {
    let (_target, db, _lease) = isolated_database().await;
    migrations::Migrator::up(db.as_ref(), Some(32))
        .await
        .unwrap();
    insert_legacy_git_state(db.as_ref()).await;
    db.execute_unprepared(CREATE_BACKFILL_TABLE).await.unwrap();
    insert_completed_backfill(db.as_ref()).await;

    migrations::Migrator::up(db.as_ref(), Some(1))
        .await
        .unwrap();

    let upload = db
        .query_one(Statement::from_string(
            DatabaseBackend::Postgres,
            format!(
                "SELECT repo_id, state, sha256, plaintext_bytes, encrypted_bytes, encoding_version
                 FROM scope_git_segment_uploads WHERE segment_id = '{SEGMENT_ID}'"
            ),
        ))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        upload.try_get::<String>("", "repo_id").unwrap(),
        REPOSITORY_ID
    );
    assert_eq!(upload.try_get::<String>("", "state").unwrap(), "published");
    assert_eq!(
        upload.try_get::<String>("", "sha256").unwrap(),
        LEGACY_SHA256
    );
    assert_eq!(upload.try_get::<i64>("", "plaintext_bytes").unwrap(), 3);
    assert_eq!(upload.try_get::<i64>("", "encrypted_bytes").unwrap(), 99);
    assert_eq!(upload.try_get::<i32>("", "encoding_version").unwrap(), 2);

    let run_source = json_column(
        db.as_ref(),
        "SELECT source AS value FROM scope_runs WHERE id = 'pinned-run'",
    )
    .await;
    let source: RunSource = serde_json::from_value(run_source.clone()).unwrap();
    assert_eq!(
        source.logical_git_head().unwrap().2[0].segment.segment_id,
        SEGMENT_ID
    );
    assert!(run_source["pack_spans"][0].get("object").is_none());

    let push_payload = json_column(
        db.as_ref(),
        "SELECT payload AS value FROM scope_outbox_jobs WHERE id = 'pending-push'",
    )
    .await;
    assert_eq!(
        push_payload["pack_spans"][0]["segment"]["segment_id"],
        SEGMENT_ID
    );
    assert!(push_payload["pack_spans"][0].get("object").is_none());

    let references = db
        .query_all(Statement::from_string(
            DatabaseBackend::Postgres,
            "SELECT ref_kind, ref_id FROM scope_git_segment_references ORDER BY ref_kind, ref_id"
                .to_string(),
        ))
        .await
        .unwrap()
        .into_iter()
        .map(|row| {
            (
                row.try_get::<String>("", "ref_kind").unwrap(),
                row.try_get::<String>("", "ref_id").unwrap(),
            )
        })
        .collect::<Vec<_>>();
    assert_eq!(
        references,
        [
            (
                "push_trigger_source".to_string(),
                format!("{REPOSITORY_ID}:1")
            ),
            ("run_source".to_string(), "pinned-run".to_string()),
        ]
    );
    assert_eq!(
        scalar_i64(
            db.as_ref(),
            "SELECT count(*) AS value FROM scope_object_references
             WHERE object_key::jsonb ? 'GitSegmentSha256'"
        )
        .await,
        0
    );
    assert_eq!(
        scalar_i64(
            db.as_ref(),
            "SELECT count(*) AS value FROM scope_object_references
             WHERE object_key::jsonb ? 'GitManifestSha256'"
        )
        .await,
        1
    );
    assert!(!table_exists(db.as_ref(), "scope_git_segment_v2_backfill").await);
    let cleanup = db
        .query_one(Statement::from_string(
            DatabaseBackend::Postgres,
            format!(
                "SELECT generation, sha256, git_oid, size_bytes, completed_at_unix
                 FROM scope_orphan_object_jobs
                 WHERE object_key = jsonb_build_object(
                     'GitSegmentSha256', '{LEGACY_SHA256}'
                 )::text"
            ),
        ))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        cleanup.try_get::<String>("", "generation").unwrap(),
        "m0033_git_segment_streaming_v2"
    );
    assert_eq!(
        cleanup.try_get::<String>("", "sha256").unwrap(),
        LEGACY_SHA256
    );
    assert_eq!(
        cleanup.try_get::<String>("", "git_oid").unwrap(),
        "b".repeat(40)
    );
    assert_eq!(cleanup.try_get::<i64>("", "size_bytes").unwrap(), 3);
    assert!(
        cleanup
            .try_get::<Option<i64>>("", "completed_at_unix")
            .unwrap()
            .is_none()
    );
}

#[tokio::test]
async fn v2_cutover_replaces_generic_object_columns_with_segment_identity() {
    let (_target, db, _lease) = isolated_database().await;
    migrations::apply_in_maintenance(db.as_ref()).await.unwrap();

    let columns = db
        .query_all(Statement::from_string(
            DatabaseBackend::Postgres,
            "SELECT column_name
             FROM information_schema.columns
             WHERE table_schema = current_schema()
               AND table_name = 'scope_git_segments'
             ORDER BY column_name"
                .to_string(),
        ))
        .await
        .unwrap()
        .into_iter()
        .map(|row| row.try_get::<String>("", "column_name").unwrap())
        .collect::<Vec<_>>();
    assert_eq!(
        columns,
        [
            "base_oid",
            "first_sequence",
            "geometric_tier",
            "head_oid",
            "last_sequence",
            "repo_id",
            "segment_id",
        ]
    );
}

async fn insert_legacy_git_state<C>(db: &C)
where
    C: ConnectionTrait,
{
    db.execute_unprepared(&format!(
        r#"
        INSERT INTO scope_users (id, handle, email, email_verified)
        VALUES ('cutover_user', 'cutover-user', 'cutover@scope.test', TRUE);
        INSERT INTO scope_repositories (
            id, owner_handle, name, owner_user_id, publication_state,
            change_version, repo_config, policy
        ) VALUES (
            '{REPOSITORY_ID}', 'cutover-user', 'repo', 'cutover_user', 'Ready', 1,
            '{{"kind":"scope.repo-config","version":1,"visibility":{{"default":"private","rules":[]}}}}'::jsonb,
            '{{"default_visibility":"Private","rules":[]}}'::jsonb
        );
        INSERT INTO scope_git_segments (
            repo_id, first_sequence, last_sequence, geometric_tier,
            base_oid, head_oid, object_key, sha256, size_bytes
        ) VALUES (
            '{REPOSITORY_ID}', 1, 1, 0, NULL, repeat('b', 40),
            jsonb_build_object('GitSegmentSha256', '{LEGACY_SHA256}')::text,
            '{LEGACY_SHA256}', 3
        );
        INSERT INTO scope_workflow_revisions (digest, definition, created_at_unix)
        VALUES (repeat('c', 64), '{{"jobs":[{{}}]}}'::jsonb, 1);
        INSERT INTO scope_runs (
            id, idempotency_key, repo_id, workflow_path, workflow_revision_digest,
            trigger, requested_by_user_id, source, state, cancellation_requested,
            created_at_unix, updated_at_unix, completed_at_unix
        ) VALUES (
            'pinned-run', 'pinned-run', '{REPOSITORY_ID}', '.scope/runs/checks.yml',
            repeat('c', 64), 'manual', 'cutover_user',
            jsonb_build_object(
                'kind', 'accepted-git-head',
                'repository_id', '{REPOSITORY_ID}',
                'head', jsonb_build_object(
                    'head_oid', repeat('b', 40), 'push_sequence', 1, 'change_version', 1,
                    'manifest', jsonb_build_object(
                        'content_ref', jsonb_build_object('GitManifestSha256', repeat('d', 64)),
                        'sha256', repeat('d', 64), 'git_oid', repeat('b', 40),
                        'git_file_mode', '100644', 'size_bytes', 4
                    )
                ),
                'pack_spans', jsonb_build_array(jsonb_build_object(
                    'first_sequence', 1, 'last_sequence', 1, 'geometric_tier', 0,
                    'base_oid', NULL, 'head_oid', repeat('b', 40),
                    'object', jsonb_build_object(
                        'content_ref', jsonb_build_object('GitSegmentSha256', '{LEGACY_SHA256}'),
                        'sha256', '{LEGACY_SHA256}', 'git_oid', repeat('b', 40),
                        'git_file_mode', '100644', 'size_bytes', 3
                    )
                )),
                'audience', 'Private'
            ),
            'succeeded', FALSE, 1, 2, 2
        );
        INSERT INTO scope_outbox_jobs (
            id, idempotency_key, kind, repo_id, repo_version, payload,
            state, attempts, next_run_at_unix, lease_owner, lease_expires_at_unix,
            last_error, created_at_unix, updated_at_unix, completed_at_unix
        ) VALUES (
            'pending-push', 'push_main_trigger_evaluation:{REPOSITORY_ID}:1',
            'push_main_trigger_evaluation', '{REPOSITORY_ID}', 1,
            jsonb_build_object(
                'workflow_schema_version', 5,
                'pack_spans', jsonb_build_array(jsonb_build_object(
                    'first_sequence', 1, 'last_sequence', 1, 'geometric_tier', 0,
                    'base_oid', NULL, 'head_oid', repeat('b', 40),
                    'object', jsonb_build_object(
                        'content_ref', jsonb_build_object('GitSegmentSha256', '{LEGACY_SHA256}'),
                        'sha256', '{LEGACY_SHA256}', 'git_oid', repeat('b', 40),
                        'git_file_mode', '100644', 'size_bytes', 3
                    )
                ))
            ),
            'ready', 0, 1, NULL, NULL, NULL, 1, 1, NULL
        );
        INSERT INTO scope_object_references (object_key, ref_kind, ref_id)
        VALUES
            (jsonb_build_object('GitSegmentSha256', '{LEGACY_SHA256}')::text,
             'git_segment', '{REPOSITORY_ID}:1'),
            (jsonb_build_object('GitSegmentSha256', '{LEGACY_SHA256}')::text,
             'run_source', 'pinned-run'),
            (jsonb_build_object('GitSegmentSha256', '{LEGACY_SHA256}')::text,
             'push_trigger_source', '{REPOSITORY_ID}:1'),
            (jsonb_build_object('GitManifestSha256', repeat('d', 64))::text,
             'run_source', 'pinned-run');
        "#
    ))
    .await
    .unwrap();
}

async fn insert_completed_backfill<C>(db: &C)
where
    C: ConnectionTrait,
{
    db.execute_unprepared(&format!(
        r#"
        INSERT INTO scope_git_segment_v2_backfill (
            repo_id, first_sequence, last_sequence,
            legacy_object_key, legacy_sha256, legacy_size_bytes,
            segment_id, object_key, sha256, plaintext_bytes,
            encrypted_bytes, encoding_version, completed_at_unix
        ) VALUES (
            '{REPOSITORY_ID}', 1, 1,
            jsonb_build_object('GitSegmentSha256', '{LEGACY_SHA256}')::text,
            '{LEGACY_SHA256}', 3, '{SEGMENT_ID}',
            'git/segments/v2/repository-hash/{SEGMENT_ID}',
            '{LEGACY_SHA256}', 3, 99, 2, 10
        );
        "#
    ))
    .await
    .unwrap();
}

async fn table_exists<C>(db: &C, table: &str) -> bool
where
    C: ConnectionTrait,
{
    db.query_one(Statement::from_sql_and_values(
        DatabaseBackend::Postgres,
        "SELECT to_regclass(format('%I.%I', current_schema(), $1)) IS NOT NULL AS present",
        [table.into()],
    ))
    .await
    .unwrap()
    .unwrap()
    .try_get::<bool>("", "present")
    .unwrap()
}

async fn column_exists<C>(db: &C, table: &str, column: &str) -> bool
where
    C: ConnectionTrait,
{
    db.query_one(Statement::from_sql_and_values(
        DatabaseBackend::Postgres,
        "SELECT EXISTS (
             SELECT 1 FROM information_schema.columns
             WHERE table_schema = current_schema() AND table_name = $1 AND column_name = $2
         ) AS present",
        [table.into(), column.into()],
    ))
    .await
    .unwrap()
    .unwrap()
    .try_get::<bool>("", "present")
    .unwrap()
}

async fn json_column<C>(db: &C, sql: &str) -> serde_json::Value
where
    C: ConnectionTrait,
{
    db.query_one(Statement::from_string(
        DatabaseBackend::Postgres,
        sql.to_string(),
    ))
    .await
    .unwrap()
    .unwrap()
    .try_get::<serde_json::Value>("", "value")
    .unwrap()
}

async fn scalar_i64<C>(db: &C, sql: &str) -> i64
where
    C: ConnectionTrait,
{
    db.query_one(Statement::from_string(
        DatabaseBackend::Postgres,
        sql.to_string(),
    ))
    .await
    .unwrap()
    .unwrap()
    .try_get::<i64>("", "value")
    .unwrap()
}
