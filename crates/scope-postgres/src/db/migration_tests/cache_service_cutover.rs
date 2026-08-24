use super::*;

const REPOSITORY_ID: &str = "cache-owner/cache-repo";
const OBJECT_DIGEST: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const IDENTITY_DIGEST: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";

#[tokio::test]
async fn cache_service_cutover_replaces_legacy_objects_without_copying_them() {
    let (_target, db, _lease) = isolated_database().await;
    migrations::Migrator::up(db.as_ref(), Some(20))
        .await
        .unwrap();
    db.execute_unprepared(&format!(
        r#"
        INSERT INTO scope_users (id, handle, email, email_verified)
        VALUES ('cache-user', 'cache-owner', 'cache@scope.test', TRUE);
        INSERT INTO scope_repositories (
            id, owner_handle, name, owner_user_id, publication_state,
            change_version, repo_config, policy
        ) VALUES (
            '{REPOSITORY_ID}', 'cache-owner', 'cache-repo', 'cache-user', 'Ready',
            1, '{{}}'::jsonb, '{{}}'::jsonb
        );
        INSERT INTO scope_run_cache_objects (
            identity_digest, object_key, checksum_sha256, size_bytes,
            generation, ready, updated_at_unix
        ) VALUES (
            '{IDENTITY_DIGEST}', 'run-caches/v1/legacy/1.tar.zst',
            '{OBJECT_DIGEST}', 128, 1, TRUE, 1
        );
        "#
    ))
    .await
    .unwrap();

    migrations::Migrator::up(db.as_ref(), Some(1))
        .await
        .unwrap();

    assert!(!relation_exists(db.as_ref(), "scope_run_cache_objects").await);
    assert!(relation_exists(db.as_ref(), "scope_run_attempt_caches").await);
    for table in [
        "scope_cache_objects",
        "scope_cache_references",
        "scope_cache_uploads",
        "scope_cache_deletion_queue",
    ] {
        assert!(relation_exists(db.as_ref(), table).await, "missing {table}");
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
        assert_eq!(count, 0, "{table} must start cold");
    }
}

#[tokio::test]
async fn cache_service_schema_enforces_content_and_lifecycle_invariants() {
    let (_target, db, _lease) = isolated_database().await;
    migrations::apply_in_maintenance(db.as_ref()).await.unwrap();
    db.execute_unprepared(&format!(
        "
        INSERT INTO scope_users (id, handle, email, email_verified)
        VALUES ('cache-user', 'cache-owner', 'cache@scope.test', TRUE);
        INSERT INTO scope_repositories (
            id, owner_handle, name, owner_user_id, publication_state,
            change_version, repo_config, policy
        ) VALUES (
            '{REPOSITORY_ID}', 'cache-owner', 'cache-repo', 'cache-user', 'Ready',
            1, '{{}}'::jsonb, '{{}}'::jsonb
        );
        INSERT INTO scope_cache_objects (
            repository_id, checksum_sha256, storage_backend, object_key,
            size_bytes, created_at_unix, last_accessed_at_unix
        ) VALUES (
            '{REPOSITORY_ID}', '{OBJECT_DIGEST}', 'railway-iad',
            'repos/{REPOSITORY_ID}/objects/sha256/{OBJECT_DIGEST}',
            1024, 10, 10
        );
        INSERT INTO scope_cache_references (
            repository_id, identity_digest, compatibility_group_digest,
            checksum_sha256, created_at_unix, expires_at_unix, last_accessed_at_unix
        ) VALUES (
            '{REPOSITORY_ID}', '{IDENTITY_DIGEST}', repeat('c', 64), '{OBJECT_DIGEST}',
            10, 604810, 10
        );
        INSERT INTO scope_cache_uploads (
            upload_id, repository_id, identity_digest, compatibility_group_digest,
            checksum_sha256, storage_backend, object_key, size_bytes,
            state, created_at_unix, expires_at_unix
        ) VALUES (
            'upload-1', '{REPOSITORY_ID}', repeat('d', 64), repeat('e', 64), repeat('f', 64),
            'railway-iad',
            'repos/{REPOSITORY_ID}/objects/sha256/' || repeat('f', 64),
            2048, 'active', 20, 1820
        );
        INSERT INTO scope_cache_deletion_queue (
            repository_id, checksum_sha256, not_before_unix, attempts, last_error
        ) VALUES ('{REPOSITORY_ID}', '{OBJECT_DIGEST}', 3610, 0, NULL);
        "
    ))
    .await
    .unwrap();

    for index in [
        "idx_scope_cache_objects_access",
        "idx_scope_cache_references_object",
        "idx_scope_cache_references_expiry",
        "idx_scope_cache_references_access",
        "idx_scope_cache_uploads_expiry",
        "idx_scope_cache_uploads_active_identity",
        "idx_scope_cache_orphan_uploads_due",
        "idx_scope_cache_deletion_queue_due",
    ] {
        assert!(relation_exists(db.as_ref(), index).await, "missing {index}");
    }

    assert!(
        db.execute_unprepared(&format!(
            "INSERT INTO scope_cache_objects (
                repository_id, checksum_sha256, storage_backend, object_key,
                size_bytes, created_at_unix, last_accessed_at_unix
            ) VALUES (
                '{REPOSITORY_ID}', repeat('A', 64), 'railway-iad',
                'repos/{REPOSITORY_ID}/objects/sha256/' || repeat('A', 64),
                1, 0, 0
            )"
        ))
        .await
        .is_err(),
        "object digests must be lowercase SHA-256"
    );
    assert!(
        db.execute_unprepared(&format!(
            "INSERT INTO scope_cache_uploads (
                upload_id, repository_id, identity_digest, compatibility_group_digest,
                checksum_sha256,
                storage_backend, object_key, size_bytes,
                state, created_at_unix, expires_at_unix
            ) VALUES (
                'oversized', '{REPOSITORY_ID}', repeat('e', 64), repeat('d', 64),
                repeat('f', 64),
                'railway-iad',
                'repos/{REPOSITORY_ID}/objects/sha256/' || repeat('f', 64),
                1073741825, 'active', 0, 1800
            )"
        ))
        .await
        .is_err(),
        "the launch object cap must be enforced"
    );
    assert!(
        db.execute_unprepared(&format!(
            "INSERT INTO scope_cache_references (
                repository_id, identity_digest, compatibility_group_digest,
                checksum_sha256, created_at_unix, expires_at_unix, last_accessed_at_unix
            ) VALUES (
                '{REPOSITORY_ID}', repeat('f', 64), repeat('d', 64), repeat('e', 64),
                10, 20, 10
            )"
        ))
        .await
        .is_err(),
        "references cannot name uncommitted objects"
    );

    db.execute_unprepared(&format!(
        "DELETE FROM scope_repositories WHERE id = '{REPOSITORY_ID}'"
    ))
    .await
    .unwrap();
    for table in [
        "scope_cache_objects",
        "scope_cache_references",
        "scope_cache_uploads",
        "scope_cache_orphan_uploads",
        "scope_cache_deletion_queue",
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
        assert_eq!(count, 0, "repository deletion must clear {table}");
    }
}

#[tokio::test]
async fn exact_compatible_cutover_preserves_objects_and_queues_physical_cleanup() {
    let (_target, db, _lease) = isolated_database().await;
    migrations::Migrator::up(db.as_ref(), Some(28))
        .await
        .unwrap();
    db.execute_unprepared(&format!(
        r#"
        INSERT INTO scope_users (id, handle, email, email_verified)
        VALUES ('cache-user', 'cache-owner', 'cache@scope.test', TRUE);
        INSERT INTO scope_repositories (
            id, owner_handle, name, owner_user_id, publication_state,
            change_version, repo_config, policy
        ) VALUES (
            '{REPOSITORY_ID}', 'cache-owner', 'cache-repo', 'cache-user', 'Ready',
            1, '{{}}'::jsonb, '{{}}'::jsonb
        );
        INSERT INTO scope_cache_objects (
            repository_id, checksum_sha256, storage_backend, object_key,
            size_bytes, created_at_unix, last_accessed_at_unix
        ) VALUES (
            '{REPOSITORY_ID}', '{OBJECT_DIGEST}', 'railway-iad',
            'repos/{REPOSITORY_ID}/objects/sha256/{OBJECT_DIGEST}',
            1024, 10, 10
        );
        INSERT INTO scope_cache_references (
            repository_id, identity_digest, checksum_sha256, version,
            expires_at_unix, last_accessed_at_unix
        ) VALUES (
            '{REPOSITORY_ID}', '{IDENTITY_DIGEST}', '{OBJECT_DIGEST}', 1, 604810, 10
        );
        INSERT INTO scope_cache_uploads (
            upload_id, repository_id, identity_digest, checksum_sha256,
            storage_backend, object_key, size_bytes, expected_reference_version,
            state, created_at_unix, expires_at_unix
        ) VALUES (
            'old-upload', '{REPOSITORY_ID}', repeat('c', 64), repeat('d', 64),
            'railway-iad',
            'repos/{REPOSITORY_ID}/objects/sha256/' || repeat('d', 64),
            2048, NULL, 'active', 20, 1820
        );
        INSERT INTO scope_cache_uploads (
            upload_id, repository_id, identity_digest, checksum_sha256,
            storage_backend, object_key, size_bytes, expected_reference_version,
            state, created_at_unix, expires_at_unix
        ) VALUES (
            'committed-upload', '{REPOSITORY_ID}', repeat('e', 64), '{OBJECT_DIGEST}',
            'railway-iad', 'repos/{REPOSITORY_ID}/objects/sha256/{OBJECT_DIGEST}',
            1024, 1, 'committed', 20, 1820
        );
        INSERT INTO scope_workflow_revisions (digest, definition, created_at_unix)
        VALUES (
            repeat('1', 64),
            '{{
                "name":"Old cache workflow",
                "triggers":{{"manual":true,"push_main":false}},
                "jobs":[{{
                    "id":"checks","needs":[],
                    "container":{{"image":"alpine@sha256:{OBJECT_DIGEST}"}},
                    "timeout_seconds":60,
                    "caches":[{{"name":"cargo","path":"/scope/cache/cargo"}}],
                    "environment":{{}},
                    "steps":[{{"name":"Test","run":"true"}}]
                }}]
            }}'::jsonb,
            1
        );
        INSERT INTO scope_runs (
            id, idempotency_key, repo_id, workflow_path, workflow_revision_digest,
            trigger, requested_by_user_id, source, state, cancellation_requested,
            created_at_unix, updated_at_unix, completed_at_unix
        ) VALUES (
            'old-cache-run', 'old-cache-run', '{REPOSITORY_ID}', '.scope/runs/checks.yml',
            repeat('1', 64), 'manual', 'cache-user',
            jsonb_build_object(
                'kind', 'ephemeral-git-bundle',
                'object', jsonb_build_object(
                    'content_ref', jsonb_build_object('GitBundleSha256', repeat('9', 64)),
                    'sha256', repeat('9', 64), 'git_oid', repeat('8', 40),
                    'git_file_mode', '100644', 'size_bytes', 32
                )
            ),
            'succeeded', FALSE, 30, 31, 31
        );
        INSERT INTO scope_object_references (object_key, ref_kind, ref_id)
        VALUES (
            jsonb_build_object('GitBundleSha256', repeat('9', 64))::text,
            'run_source', 'old-cache-run'
        );
        INSERT INTO scope_repository_workflow_catalogs (
            repo_id, source_head_oid, source_change_version, configuration_error
        ) VALUES ('{REPOSITORY_ID}', repeat('8', 40), 1, NULL);
        INSERT INTO scope_repository_workflow_files (
            repo_id, path, oid, size_bytes, git_file_mode, content_bytes
        ) VALUES (
            '{REPOSITORY_ID}', '/.scope/runs/checks.yml', repeat('7', 40),
            18, '100644', convert_to(E'caches:\n  - cargo\n', 'UTF8')
        );
        "#
    ))
    .await
    .unwrap();

    migrations::Migrator::up(db.as_ref(), Some(1))
        .await
        .unwrap();

    for (table, expected) in [
        ("scope_cache_objects", 1_i64),
        ("scope_cache_references", 0),
        ("scope_cache_uploads", 0),
        ("scope_cache_orphan_uploads", 1),
        ("scope_cache_deletion_queue", 1),
        ("scope_runs", 0),
        ("scope_workflow_revisions", 0),
        ("scope_repository_workflow_catalogs", 0),
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
        assert_eq!(count, expected, "unexpected rows in {table}");
    }
    let obsolete_columns = db
        .query_one(Statement::from_string(
            DatabaseBackend::Postgres,
            "SELECT count(*) AS count FROM information_schema.columns
             WHERE table_schema = current_schema()
               AND column_name IN ('version', 'expected_reference_version')
               AND table_name IN ('scope_cache_references', 'scope_cache_uploads')"
                .to_string(),
        ))
        .await
        .unwrap()
        .unwrap()
        .try_get::<i64>("", "count")
        .unwrap();
    assert_eq!(obsolete_columns, 0);

    let retired_source_jobs = db
        .query_one(Statement::from_string(
            DatabaseBackend::Postgres,
            "SELECT count(*) AS count FROM scope_orphan_object_jobs
             WHERE generation = 'm0029_exact_compatible_caches'"
                .to_string(),
        ))
        .await
        .unwrap()
        .unwrap()
        .try_get::<i64>("", "count")
        .unwrap();
    assert_eq!(retired_source_jobs, 1);

    let caches = crate::db::CacheStore { db: db.clone() };
    let first_claim = caches
        .claim_orphan_uploads(4_000_000_000, 4_000_000_300, 10)
        .await
        .unwrap();
    assert_eq!(first_claim.len(), 1);
    assert_eq!(
        first_claim[0].object_key,
        format!("repos/{REPOSITORY_ID}/objects/sha256/{}", "d".repeat(64))
    );
    caches
        .fail_orphan_upload_cleanup(
            &first_claim[0].object_key,
            4_000_000_300,
            "temporary failure",
        )
        .await
        .unwrap();
    assert!(
        caches
            .claim_orphan_uploads(4_000_000_299, 4_000_000_600, 10)
            .await
            .unwrap()
            .is_empty()
    );
    let retry = caches
        .claim_orphan_uploads(4_000_000_300, 4_000_000_600, 10)
        .await
        .unwrap();
    assert_eq!(retry[0].attempts, 2);
    caches
        .complete_orphan_upload_cleanup(&retry[0].object_key)
        .await
        .unwrap();
    assert!(
        caches
            .claim_orphan_uploads(4_000_001_000, 4_000_001_300, 10)
            .await
            .unwrap()
            .is_empty()
    );
}
