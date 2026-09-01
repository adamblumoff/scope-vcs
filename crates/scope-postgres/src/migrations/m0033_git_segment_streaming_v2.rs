use sea_orm::ConnectionTrait;
use sea_orm_migration::{DbErr, MigrationName, MigrationTrait, SchemaManager};

pub struct Migration;

impl MigrationName for Migration {
    fn name(&self) -> &str {
        "m0033_git_segment_streaming_v2"
    }
}

#[sea_orm_migration::async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let connection = manager.get_connection();
        connection
            .execute_unprepared(crate::db::git_segment_v2_backfill::CREATE_BACKFILL_TABLE)
            .await?;
        connection
            .execute_unprepared(
                r#"
                LOCK TABLE scope_git_segments, scope_object_references,
                    scope_runs, scope_outbox_jobs, scope_orphan_object_jobs,
                    scope_git_segment_v2_backfill
                    IN ACCESS EXCLUSIVE MODE;

                CREATE TEMP TABLE scope_git_segment_v2_sources ON COMMIT DROP AS
                SELECT repo_id, first_sequence, last_sequence,
                       (object_key::jsonb)::text AS legacy_object_key,
                       sha256 AS legacy_sha256, size_bytes AS legacy_size_bytes,
                       geometric_tier, base_oid, head_oid
                FROM scope_git_segments
                UNION ALL
                SELECT runs.source->>'repository_id',
                       (span->>'first_sequence')::bigint,
                       (span->>'last_sequence')::bigint,
                       (span#>'{object,content_ref}')::text,
                       span#>>'{object,sha256}',
                       (span#>>'{object,size_bytes}')::bigint,
                       (span->>'geometric_tier')::integer,
                       span->>'base_oid', span->>'head_oid'
                FROM scope_runs runs
                CROSS JOIN LATERAL jsonb_array_elements(runs.source->'pack_spans') span
                WHERE runs.source->>'kind' = 'accepted-git-head'
                UNION ALL
                SELECT jobs.repo_id,
                       (span->>'first_sequence')::bigint,
                       (span->>'last_sequence')::bigint,
                       (span#>'{object,content_ref}')::text,
                       span#>>'{object,sha256}',
                       (span#>>'{object,size_bytes}')::bigint,
                       (span->>'geometric_tier')::integer,
                       span->>'base_oid', span->>'head_oid'
                FROM scope_outbox_jobs jobs
                CROSS JOIN LATERAL jsonb_array_elements(jobs.payload->'pack_spans') span
                WHERE jobs.kind = 'push_main_trigger_evaluation'
                  AND jobs.completed_at_unix IS NULL;

                DO $$
                BEGIN
                    IF EXISTS (
                        SELECT 1
                        FROM scope_git_segment_v2_sources spans
                        WHERE spans.repo_id IS NULL
                           OR length(btrim(spans.repo_id)) = 0
                           OR spans.first_sequence IS NULL
                           OR spans.first_sequence <= 0
                           OR spans.last_sequence IS NULL
                           OR spans.last_sequence < spans.first_sequence
                           OR spans.geometric_tier IS NULL
                           OR spans.geometric_tier NOT BETWEEN 0 AND 62
                           OR spans.last_sequence - spans.first_sequence + 1 <>
                              power(2::numeric, spans.geometric_tier)
                           OR spans.legacy_sha256 IS NULL
                           OR length(spans.legacy_sha256) <> 64
                           OR spans.legacy_sha256 !~ '^[0-9a-f]+$'
                           OR spans.legacy_size_bytes IS NULL
                           OR spans.legacy_size_bytes < 0
                           OR spans.legacy_object_key IS NULL
                           OR spans.legacy_object_key::jsonb <>
                              jsonb_build_object('GitSegmentSha256', spans.legacy_sha256)
                           OR spans.head_oid IS NULL
                           OR length(btrim(spans.head_oid)) = 0
                    ) THEN
                        RAISE EXCEPTION 'm0033 found invalid legacy Git segment metadata';
                    END IF;

                    IF EXISTS (
                        SELECT 1
                        FROM scope_git_segment_v2_sources
                        GROUP BY repo_id, first_sequence, last_sequence, legacy_object_key
                        HAVING count(DISTINCT jsonb_build_object(
                            'sha256', legacy_sha256,
                            'size_bytes', legacy_size_bytes,
                            'geometric_tier', geometric_tier,
                            'base_oid', base_oid,
                            'head_oid', head_oid
                        )) > 1
                    ) THEN
                        RAISE EXCEPTION
                            'm0033 found conflicting metadata for a legacy Git segment identity';
                    END IF;

                    IF EXISTS (
                        SELECT 1
                        FROM scope_git_segment_v2_sources spans
                        LEFT JOIN scope_git_segment_v2_backfill prepared
                          ON prepared.repo_id = spans.repo_id
                         AND prepared.first_sequence = spans.first_sequence
                         AND prepared.last_sequence = spans.last_sequence
                         AND prepared.legacy_object_key = spans.legacy_object_key
                         AND prepared.legacy_sha256 = spans.legacy_sha256
                         AND prepared.legacy_size_bytes = spans.legacy_size_bytes
                        WHERE prepared.repo_id IS NULL
                    ) THEN
                        RAISE EXCEPTION USING
                            MESSAGE = 'm0033 requires every legacy Git segment to be backfilled',
                            HINT = 'Run scope-maintenance backfill-git-segments-v2 before applying migrations.';
                    END IF;

                    IF EXISTS (
                        SELECT 1
                        FROM scope_git_segment_v2_backfill prepared
                        LEFT JOIN scope_git_segment_v2_sources spans
                          ON prepared.repo_id = spans.repo_id
                         AND prepared.first_sequence = spans.first_sequence
                         AND prepared.last_sequence = spans.last_sequence
                         AND prepared.legacy_object_key = spans.legacy_object_key
                         AND prepared.legacy_sha256 = spans.legacy_sha256
                         AND prepared.legacy_size_bytes = spans.legacy_size_bytes
                        WHERE spans.repo_id IS NULL
                    ) THEN
                        RAISE EXCEPTION 'm0033 found a stale Git segment backfill record';
                    END IF;
                END
                $$;

                CREATE TABLE scope_git_segment_uploads (
                    segment_id text PRIMARY KEY,
                    repo_id text NOT NULL,
                    object_key text NOT NULL UNIQUE,
                    state text NOT NULL,
                    sha256 text,
                    plaintext_bytes bigint,
                    encrypted_bytes bigint,
                    encoding_version integer NOT NULL,
                    created_at_unix bigint NOT NULL,
                    updated_at_unix bigint NOT NULL,
                    CONSTRAINT scope_git_segment_upload_state CHECK (
                        state IN (
                            'uploading', 'ready', 'published', 'retained', 'deleting', 'deleted'
                        )
                    ),
                    CONSTRAINT scope_git_segment_upload_values CHECK (
                        length(btrim(segment_id)) > 0 AND
                        length(btrim(object_key)) > 0 AND
                        encoding_version > 0 AND
                        created_at_unix >= 0 AND
                        updated_at_unix >= created_at_unix AND
                        (sha256 IS NULL OR length(sha256) = 64) AND
                        (plaintext_bytes IS NULL OR plaintext_bytes >= 0) AND
                        (encrypted_bytes IS NULL OR encrypted_bytes >= 0) AND
                        (
                            state NOT IN ('ready', 'published', 'retained') OR
                            (sha256 IS NOT NULL AND plaintext_bytes IS NOT NULL AND encrypted_bytes IS NOT NULL)
                        )
                    )
                );

                CREATE INDEX idx_scope_git_segment_uploads_recovery
                    ON scope_git_segment_uploads (state, updated_at_unix, segment_id)
                    WHERE state IN ('uploading', 'ready', 'deleting');

                CREATE TABLE scope_git_segment_references (
                    segment_id text NOT NULL
                        REFERENCES scope_git_segment_uploads(segment_id) ON DELETE RESTRICT,
                    ref_kind text NOT NULL,
                    ref_id text NOT NULL,
                    PRIMARY KEY (segment_id, ref_kind, ref_id),
                    CONSTRAINT scope_git_segment_reference_values CHECK (
                        ref_kind IN ('push_trigger_source', 'run_source') AND
                        length(btrim(ref_id)) > 0
                    )
                );

                CREATE INDEX idx_scope_git_segment_references_owner
                    ON scope_git_segment_references (ref_kind, ref_id, segment_id);

                INSERT INTO scope_git_segment_uploads (
                    segment_id, repo_id, object_key, state, sha256,
                    plaintext_bytes, encrypted_bytes, encoding_version,
                    created_at_unix, updated_at_unix
                )
                SELECT prepared.segment_id, prepared.repo_id, prepared.object_key,
                       CASE WHEN EXISTS (
                           SELECT 1 FROM scope_git_segments spans
                           WHERE spans.repo_id = prepared.repo_id
                             AND spans.first_sequence = prepared.first_sequence
                             AND spans.last_sequence = prepared.last_sequence
                             AND (spans.object_key::jsonb)::text =
                                 prepared.legacy_object_key
                             AND spans.sha256 = prepared.legacy_sha256
                             AND spans.size_bytes = prepared.legacy_size_bytes
                       ) THEN 'published' ELSE 'retained' END,
                       prepared.sha256,
                       plaintext_bytes, encrypted_bytes, encoding_version,
                       completed_at_unix, completed_at_unix
                FROM scope_git_segment_v2_backfill prepared;

                ALTER TABLE scope_git_segments
                    ADD COLUMN segment_id text;

                UPDATE scope_git_segments spans
                SET segment_id = prepared.segment_id
                FROM scope_git_segment_v2_backfill prepared
                WHERE prepared.repo_id = spans.repo_id
                  AND prepared.first_sequence = spans.first_sequence
                  AND prepared.last_sequence = spans.last_sequence
                  AND prepared.legacy_object_key = (spans.object_key::jsonb)::text
                  AND prepared.legacy_sha256 = spans.sha256
                  AND prepared.legacy_size_bytes = spans.size_bytes;

                ALTER TABLE scope_git_segments
                    ALTER COLUMN segment_id SET NOT NULL,
                    DROP COLUMN object_key,
                    DROP COLUMN sha256,
                    DROP COLUMN size_bytes,
                    ADD CONSTRAINT fk_scope_git_segments_upload
                        FOREIGN KEY (segment_id)
                        REFERENCES scope_git_segment_uploads(segment_id),
                    ADD CONSTRAINT uq_scope_git_segments_segment UNIQUE (segment_id);

                UPDATE scope_runs runs
                SET source = jsonb_set(
                    runs.source,
                    '{pack_spans}',
                    (
                        SELECT jsonb_agg(
                            (span - 'object') || jsonb_build_object(
                                'segment', jsonb_build_object(
                                    'segment_id', prepared.segment_id,
                                    'sha256', prepared.sha256,
                                    'plaintext_bytes', prepared.plaintext_bytes,
                                    'encoding_version', prepared.encoding_version
                                )
                            )
                            ORDER BY ordinal
                        )
                        FROM jsonb_array_elements(runs.source->'pack_spans')
                            WITH ORDINALITY spans(span, ordinal)
                        JOIN scope_git_segment_v2_backfill prepared
                          ON prepared.repo_id = runs.source->>'repository_id'
                         AND prepared.first_sequence = (span->>'first_sequence')::bigint
                         AND prepared.last_sequence = (span->>'last_sequence')::bigint
                         AND prepared.legacy_object_key =
                             (span#>'{object,content_ref}')::text
                         AND prepared.legacy_sha256 = span#>>'{object,sha256}'
                         AND prepared.legacy_size_bytes =
                             (span#>>'{object,size_bytes}')::bigint
                    )
                )
                WHERE runs.source->>'kind' = 'accepted-git-head';

                UPDATE scope_outbox_jobs jobs
                SET payload = jsonb_set(
                    jobs.payload,
                    '{pack_spans}',
                    (
                        SELECT jsonb_agg(
                            (span - 'object') || jsonb_build_object(
                                'segment', jsonb_build_object(
                                    'segment_id', prepared.segment_id,
                                    'sha256', prepared.sha256,
                                    'plaintext_bytes', prepared.plaintext_bytes,
                                    'encoding_version', prepared.encoding_version
                                )
                            )
                            ORDER BY ordinal
                        )
                        FROM jsonb_array_elements(jobs.payload->'pack_spans')
                            WITH ORDINALITY spans(span, ordinal)
                        JOIN scope_git_segment_v2_backfill prepared
                          ON prepared.repo_id = jobs.repo_id
                         AND prepared.first_sequence = (span->>'first_sequence')::bigint
                         AND prepared.last_sequence = (span->>'last_sequence')::bigint
                         AND prepared.legacy_object_key =
                             (span#>'{object,content_ref}')::text
                         AND prepared.legacy_sha256 = span#>>'{object,sha256}'
                         AND prepared.legacy_size_bytes =
                             (span#>>'{object,size_bytes}')::bigint
                    )
                )
                WHERE jobs.kind = 'push_main_trigger_evaluation'
                  AND jobs.completed_at_unix IS NULL;

                INSERT INTO scope_git_segment_references (segment_id, ref_kind, ref_id)
                SELECT DISTINCT span#>>'{segment,segment_id}', 'run_source', runs.id
                FROM scope_runs runs
                CROSS JOIN LATERAL jsonb_array_elements(runs.source->'pack_spans') span
                WHERE runs.source->>'kind' = 'accepted-git-head'
                ON CONFLICT DO NOTHING;

                INSERT INTO scope_git_segment_references (segment_id, ref_kind, ref_id)
                SELECT DISTINCT span#>>'{segment,segment_id}', 'push_trigger_source',
                       jobs.repo_id || ':' || jobs.repo_version::text
                FROM scope_outbox_jobs jobs
                CROSS JOIN LATERAL jsonb_array_elements(jobs.payload->'pack_spans') span
                WHERE jobs.kind = 'push_main_trigger_evaluation'
                  AND jobs.completed_at_unix IS NULL
                ON CONFLICT DO NOTHING;

                INSERT INTO scope_orphan_object_jobs (
                    object_key, generation, sha256, git_oid, size_bytes,
                    attempts, next_run_at_unix, last_error, completed_at_unix,
                    created_at_unix, updated_at_unix
                )
                SELECT DISTINCT ON (prepared.legacy_object_key)
                    prepared.legacy_object_key, 'm0033_git_segment_streaming_v2',
                    prepared.legacy_sha256, sources.head_oid,
                    prepared.legacy_size_bytes, 0,
                    EXTRACT(EPOCH FROM clock_timestamp())::bigint,
                    NULL, NULL, 0, 0
                FROM scope_git_segment_v2_backfill prepared
                JOIN scope_git_segment_v2_sources sources
                  ON sources.repo_id = prepared.repo_id
                 AND sources.first_sequence = prepared.first_sequence
                 AND sources.last_sequence = prepared.last_sequence
                 AND sources.legacy_object_key = prepared.legacy_object_key
                 AND sources.legacy_sha256 = prepared.legacy_sha256
                 AND sources.legacy_size_bytes = prepared.legacy_size_bytes
                ORDER BY prepared.legacy_object_key, prepared.repo_id,
                         prepared.first_sequence, prepared.last_sequence
                ON CONFLICT (object_key) DO UPDATE SET
                    generation = EXCLUDED.generation,
                    sha256 = EXCLUDED.sha256,
                    git_oid = EXCLUDED.git_oid,
                    size_bytes = EXCLUDED.size_bytes,
                    attempts = 0,
                    next_run_at_unix = EXCLUDED.next_run_at_unix,
                    last_error = NULL,
                    completed_at_unix = NULL,
                    updated_at_unix = EXCLUDED.updated_at_unix;

                DELETE FROM scope_object_references
                WHERE ref_kind = 'git_segment'
                   OR (
                        ref_kind IN ('run_source', 'push_trigger_source')
                        AND object_key::jsonb ? 'GitSegmentSha256'
                   );

                DROP TABLE scope_git_segment_v2_backfill;
                "#,
            )
            .await?;
        Ok(())
    }
}
