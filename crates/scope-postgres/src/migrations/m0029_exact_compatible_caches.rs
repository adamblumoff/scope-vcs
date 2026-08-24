use sea_orm::ConnectionTrait;
use sea_orm_migration::{DbErr, MigrationName, MigrationTrait, SchemaManager};

pub struct Migration;

impl MigrationName for Migration {
    fn name(&self) -> &str {
        "m0029_exact_compatible_caches"
    }
}

#[sea_orm_migration::async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager.get_connection().execute_unprepared(r#"
            LOCK TABLE scope_runs, scope_workflow_revisions, scope_outbox_jobs,
                scope_repository_workflow_catalogs, scope_object_references,
                scope_orphan_object_jobs, scope_cache_objects, scope_cache_references,
                scope_cache_uploads, scope_cache_deletion_queue
                IN ACCESS EXCLUSIVE MODE;

            INSERT INTO scope_orphan_object_jobs (
                object_key, generation, sha256, git_oid, size_bytes,
                attempts, next_run_at_unix, last_error, completed_at_unix,
                created_at_unix, updated_at_unix
            )
            SELECT DISTINCT ON (refs.object_key)
                refs.object_key, 'm0029_exact_compatible_caches',
                runs.source#>>'{object,sha256}', runs.source#>>'{object,git_oid}',
                (runs.source#>>'{object,size_bytes}')::bigint,
                0, EXTRACT(EPOCH FROM clock_timestamp())::bigint,
                NULL, NULL, 0, 0
            FROM scope_runs runs
            JOIN scope_object_references refs
              ON refs.ref_kind = 'run_source'
             AND refs.ref_id = runs.id
             AND refs.object_key::jsonb = runs.source#>'{object,content_ref}'
            WHERE runs.source->>'kind' = 'ephemeral-git-bundle'
              AND NOT EXISTS (
                  SELECT 1
                  FROM scope_object_references other_refs
                  WHERE other_refs.object_key = refs.object_key
                    AND NOT (
                        other_refs.ref_kind = 'run_source'
                        AND EXISTS (
                            SELECT 1 FROM scope_runs other_runs
                            WHERE other_runs.id = other_refs.ref_id
                        )
                    )
              )
            ORDER BY refs.object_key, runs.id
            ON CONFLICT (object_key) DO NOTHING;
            DELETE FROM scope_object_references refs
            WHERE refs.ref_kind = 'run_source'
              AND EXISTS (SELECT 1 FROM scope_runs runs WHERE runs.id = refs.ref_id);
            DELETE FROM scope_object_references refs
            WHERE refs.ref_kind = 'push_trigger_source'
              AND EXISTS (
                  SELECT 1 FROM scope_outbox_jobs jobs
                  WHERE jobs.kind = 'push_main_trigger_evaluation'
                    AND jobs.completed_at_unix IS NULL
                    AND refs.ref_id = jobs.repo_id || ':' || jobs.repo_version::text
              );
            DELETE FROM scope_outbox_jobs
            WHERE kind = 'push_main_trigger_evaluation'
              AND completed_at_unix IS NULL;
            TRUNCATE TABLE scope_push_trigger_evaluations,
                scope_run_attempt_caches, scope_run_logs, scope_run_attempt_steps,
                scope_run_attempts, scope_run_jobs, scope_runs,
                scope_workflow_revisions, scope_repository_workflow_catalogs CASCADE;

            ALTER TABLE scope_run_attempt_caches
                DROP CONSTRAINT scope_run_attempt_caches_preparation;
            ALTER TABLE scope_run_attempt_caches
                ADD CONSTRAINT scope_run_attempt_caches_preparation CHECK (
                    ((preparation IN ('exact', 'compatible') AND cold_reason IS NULL) OR
                     (preparation = 'cold' AND cold_reason IN (
                        'metadata-missing', 'metadata-invalid', 'metadata-not-ready',
                        'volume-missing', 'volume-invalid', 'backing-directory-missing'
                     )))
                );

            CREATE TABLE scope_cache_orphan_uploads (
                object_key text PRIMARY KEY,
                repository_id text NOT NULL REFERENCES scope_repositories(id) ON DELETE CASCADE,
                not_before_unix bigint NOT NULL,
                attempts integer NOT NULL,
                last_error text,
                CONSTRAINT scope_cache_orphan_uploads_values CHECK (
                    object_key = 'repos/' || repository_id || '/objects/sha256/' ||
                        right(object_key, 64) AND
                    right(object_key, 64) ~ '^[0-9a-f]{64}$' AND
                    not_before_unix >= 0 AND attempts >= 0 AND
                    (last_error IS NULL OR char_length(last_error) BETWEEN 1 AND 8192)
                )
            );
            CREATE INDEX idx_scope_cache_orphan_uploads_due
                ON scope_cache_orphan_uploads (not_before_unix, object_key);
            INSERT INTO scope_cache_orphan_uploads (
                object_key, repository_id, not_before_unix, attempts, last_error
            )
            SELECT uploads.object_key, uploads.repository_id,
                   EXTRACT(EPOCH FROM clock_timestamp())::bigint, 0, NULL
            FROM scope_cache_uploads uploads
            WHERE NOT EXISTS (
                SELECT 1 FROM scope_cache_objects objects
                WHERE objects.repository_id = uploads.repository_id
                  AND objects.checksum_sha256 = uploads.checksum_sha256
            );

            DROP TABLE scope_cache_deletion_queue;
            DROP TABLE scope_cache_uploads;
            DROP TABLE scope_cache_references;

            CREATE TABLE scope_cache_references (
                repository_id text NOT NULL,
                identity_digest varchar(64) NOT NULL,
                compatibility_group_digest varchar(64) NOT NULL,
                checksum_sha256 varchar(64) NOT NULL,
                created_at_unix bigint NOT NULL,
                expires_at_unix bigint NOT NULL,
                last_accessed_at_unix bigint NOT NULL,
                PRIMARY KEY (repository_id, identity_digest),
                CONSTRAINT fk_scope_cache_references_object
                    FOREIGN KEY (repository_id, checksum_sha256)
                    REFERENCES scope_cache_objects (repository_id, checksum_sha256) ON DELETE CASCADE,
                CONSTRAINT scope_cache_references_values CHECK (
                    identity_digest ~ '^[0-9a-f]{64}$' AND
                    compatibility_group_digest ~ '^[0-9a-f]{64}$' AND
                    checksum_sha256 ~ '^[0-9a-f]{64}$' AND
                    created_at_unix >= 0 AND last_accessed_at_unix >= created_at_unix AND
                    expires_at_unix > last_accessed_at_unix
                )
            );
            CREATE INDEX idx_scope_cache_references_object
                ON scope_cache_references (repository_id, checksum_sha256);
            CREATE INDEX idx_scope_cache_references_expiry
                ON scope_cache_references (expires_at_unix, repository_id, identity_digest);
            CREATE INDEX idx_scope_cache_references_access
                ON scope_cache_references (repository_id, last_accessed_at_unix, identity_digest);
            CREATE INDEX idx_scope_cache_references_compatibility
                ON scope_cache_references (
                    repository_id, compatibility_group_digest, created_at_unix DESC, identity_digest
                );

            CREATE TABLE scope_cache_uploads (
                upload_id text PRIMARY KEY,
                repository_id text NOT NULL REFERENCES scope_repositories(id) ON DELETE CASCADE,
                identity_digest varchar(64) NOT NULL,
                compatibility_group_digest varchar(64) NOT NULL,
                checksum_sha256 varchar(64) NOT NULL,
                storage_backend varchar(64) NOT NULL,
                object_key text NOT NULL UNIQUE,
                size_bytes bigint NOT NULL,
                state text NOT NULL,
                created_at_unix bigint NOT NULL,
                expires_at_unix bigint NOT NULL,
                CONSTRAINT scope_cache_uploads_values CHECK (
                    char_length(upload_id) BETWEEN 1 AND 128 AND upload_id !~ '[[:space:]]' AND
                    identity_digest ~ '^[0-9a-f]{64}$' AND
                    compatibility_group_digest ~ '^[0-9a-f]{64}$' AND
                    checksum_sha256 ~ '^[0-9a-f]{64}$' AND
                    char_length(storage_backend) BETWEEN 1 AND 64 AND
                    storage_backend ~ '^[a-z0-9]+(-[a-z0-9]+)*$' AND
                    object_key = 'repos/' || repository_id || '/objects/sha256/' || checksum_sha256 AND
                    size_bytes BETWEEN 1 AND 1073741824 AND
                    state IN ('active', 'deleting', 'committed') AND
                    created_at_unix >= 0 AND expires_at_unix > created_at_unix AND
                    expires_at_unix <= created_at_unix + 1800
                )
            );
            CREATE INDEX idx_scope_cache_uploads_expiry
                ON scope_cache_uploads (expires_at_unix, upload_id);
            CREATE UNIQUE INDEX idx_scope_cache_uploads_active_identity
                ON scope_cache_uploads (repository_id, identity_digest)
                WHERE state IN ('active', 'deleting');

            CREATE TABLE scope_cache_deletion_queue (
                repository_id text NOT NULL,
                checksum_sha256 varchar(64) NOT NULL,
                not_before_unix bigint NOT NULL,
                attempts integer NOT NULL,
                last_error text,
                PRIMARY KEY (repository_id, checksum_sha256),
                CONSTRAINT fk_scope_cache_deletion_queue_object
                    FOREIGN KEY (repository_id, checksum_sha256)
                    REFERENCES scope_cache_objects (repository_id, checksum_sha256) ON DELETE CASCADE,
                CONSTRAINT scope_cache_deletion_queue_values CHECK (
                    checksum_sha256 ~ '^[0-9a-f]{64}$' AND not_before_unix >= 0 AND attempts >= 0 AND
                    (last_error IS NULL OR char_length(last_error) BETWEEN 1 AND 8192)
                )
            );
            CREATE INDEX idx_scope_cache_deletion_queue_due
                ON scope_cache_deletion_queue (not_before_unix, repository_id, checksum_sha256);
            INSERT INTO scope_cache_deletion_queue (
                repository_id, checksum_sha256, not_before_unix, attempts, last_error
            )
            SELECT repository_id, checksum_sha256,
                   EXTRACT(EPOCH FROM clock_timestamp())::bigint, 0, NULL
            FROM scope_cache_objects;
        "#).await?;
        Ok(())
    }
}
