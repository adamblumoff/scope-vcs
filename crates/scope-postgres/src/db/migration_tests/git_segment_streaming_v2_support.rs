use sea_orm::{ConnectionTrait, DatabaseBackend, Statement};

pub(super) const REPOSITORY_ID: &str = "cutover-user/repo";
pub(super) const LEGACY_SHA256: &str =
    "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
pub(super) const SEGMENT_ID: &str = "11111111111111111111111111111111";
pub(super) const LIVE_SHA256: &str =
    "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
pub(super) const LIVE_SEGMENT_ID: &str = "22222222222222222222222222222222";

pub(super) async fn insert_legacy_git_state<C>(db: &C)
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
            '{{"GitSegmentSha256":"{LEGACY_SHA256}"}}',
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

pub(super) async fn insert_compacted_legacy_git_state<C>(db: &C)
where
    C: ConnectionTrait,
{
    insert_legacy_git_state(db).await;
    db.execute_unprepared(&format!(
        r#"
        UPDATE scope_git_segments
        SET last_sequence = 2,
            geometric_tier = 1,
            object_key = jsonb_build_object('GitSegmentSha256', '{LIVE_SHA256}')::text,
            sha256 = '{LIVE_SHA256}',
            size_bytes = 4
        WHERE repo_id = '{REPOSITORY_ID}' AND first_sequence = 1;
        INSERT INTO scope_object_references (object_key, ref_kind, ref_id)
        VALUES (
            jsonb_build_object('GitSegmentSha256', '{LIVE_SHA256}')::text,
            'git_segment', '{REPOSITORY_ID}:1'
        );
        "#
    ))
    .await
    .unwrap();
}

pub(super) async fn insert_completed_backfill<C>(db: &C)
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

pub(super) async fn insert_compacted_backfills<C>(db: &C)
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
        ) VALUES
        (
            '{REPOSITORY_ID}', 1, 1,
            jsonb_build_object('GitSegmentSha256', '{LEGACY_SHA256}')::text,
            '{LEGACY_SHA256}', 3, '{SEGMENT_ID}',
            'git/segments/v2/repository-hash/{SEGMENT_ID}',
            '{LEGACY_SHA256}', 3, 99, 2, 10
        ),
        (
            '{REPOSITORY_ID}', 1, 2,
            jsonb_build_object('GitSegmentSha256', '{LIVE_SHA256}')::text,
            '{LIVE_SHA256}', 4, '{LIVE_SEGMENT_ID}',
            'git/segments/v2/repository-hash/{LIVE_SEGMENT_ID}',
            '{LIVE_SHA256}', 4, 100, 2, 11
        );
        "#
    ))
    .await
    .unwrap();
}

pub(super) async fn table_exists<C>(db: &C, table: &str) -> bool
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

pub(super) async fn column_exists<C>(db: &C, table: &str, column: &str) -> bool
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

pub(super) async fn json_column<C>(db: &C, sql: &str) -> serde_json::Value
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

pub(super) async fn scalar_i64<C>(db: &C, sql: &str) -> i64
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
