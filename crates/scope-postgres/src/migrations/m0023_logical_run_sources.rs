use sea_orm::ConnectionTrait;
use sea_orm_migration::{DbErr, MigrationName, MigrationTrait, SchemaManager};

pub struct Migration;

impl MigrationName for Migration {
    fn name(&self) -> &str {
        "m0023_logical_run_sources"
    }
}

#[sea_orm_migration::async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .get_connection()
            .execute_unprepared(
                r#"
                LOCK TABLE scope_runs, scope_outbox_jobs, scope_push_trigger_evaluations,
                    scope_object_references, scope_orphan_object_jobs
                    IN ACCESS EXCLUSIVE MODE;

                WITH legacy_jobs AS (
                    SELECT repo_id, repo_version, payload
                    FROM scope_outbox_jobs
                    WHERE kind = 'push_main_trigger_evaluation'
                      AND completed_at_unix IS NULL
                      AND payload @> '{"workflow_schema_version": 4}'::jsonb
                ), legacy_sources AS (
                    SELECT jobs.repo_id, jobs.repo_version, sources.object
                    FROM legacy_jobs jobs
                    CROSS JOIN LATERAL (
                        VALUES (jobs.payload->'manifest'), (jobs.payload#>'{input,snapshot}')
                    ) AS sources(object)
                    WHERE sources.object IS NOT NULL
                )
                INSERT INTO scope_orphan_object_jobs (
                    object_key, generation, sha256, git_oid, size_bytes,
                    attempts, next_run_at_unix, last_error, completed_at_unix,
                    created_at_unix, updated_at_unix
                )
                SELECT DISTINCT ON (refs.object_key)
                    refs.object_key,
                    'm0023_logical_run_sources',
                    sources.object->>'sha256',
                    sources.object->>'git_oid',
                    (sources.object->>'size_bytes')::bigint,
                    0, 0, NULL, NULL, 0, 0
                FROM legacy_sources sources
                JOIN scope_object_references refs
                  ON refs.ref_kind = 'push_trigger_source'
                 AND refs.ref_id = sources.repo_id || ':' || sources.repo_version::text
                 AND refs.object_key::jsonb = sources.object->'content_ref'
                WHERE NOT EXISTS (
                    SELECT 1
                    FROM scope_object_references other_refs
                    WHERE other_refs.object_key = refs.object_key
                      AND NOT (
                          other_refs.ref_kind = 'push_trigger_source'
                          AND EXISTS (
                              SELECT 1
                              FROM legacy_jobs other_jobs
                              WHERE other_refs.ref_id =
                                  other_jobs.repo_id || ':' || other_jobs.repo_version::text
                          )
                      )
                )
                ORDER BY refs.object_key
                ON CONFLICT (object_key) DO NOTHING;

                WITH legacy_jobs AS (
                    SELECT repo_id, repo_version
                    FROM scope_outbox_jobs
                    WHERE kind = 'push_main_trigger_evaluation'
                      AND completed_at_unix IS NULL
                      AND payload @> '{"workflow_schema_version": 4}'::jsonb
                ), cutover_time AS (
                    SELECT extract(epoch FROM clock_timestamp())::bigint AS now_unix
                )
                UPDATE scope_push_trigger_evaluations evaluations
                SET state = 'failed',
                    message = 'retired by the logical Git source cutover; a later push will reevaluate workflows',
                    completed_at_unix = GREATEST(evaluations.created_at_unix, cutover_time.now_unix)
                FROM legacy_jobs, cutover_time
                WHERE evaluations.repo_id = legacy_jobs.repo_id
                  AND evaluations.change_version = legacy_jobs.repo_version
                  AND evaluations.state = 'pending';

                DELETE FROM scope_object_references refs
                WHERE refs.ref_kind = 'push_trigger_source'
                  AND EXISTS (
                      SELECT 1
                      FROM scope_outbox_jobs jobs
                      WHERE jobs.kind = 'push_main_trigger_evaluation'
                        AND jobs.completed_at_unix IS NULL
                        AND jobs.payload @> '{"workflow_schema_version": 4}'::jsonb
                        AND refs.ref_id = jobs.repo_id || ':' || jobs.repo_version::text
                  );

                DELETE FROM scope_outbox_jobs
                WHERE kind = 'push_main_trigger_evaluation'
                  AND completed_at_unix IS NULL
                  AND payload @> '{"workflow_schema_version": 4}'::jsonb;

                INSERT INTO scope_orphan_object_jobs (
                    object_key, generation, sha256, git_oid, size_bytes,
                    attempts, next_run_at_unix, last_error, completed_at_unix,
                    created_at_unix, updated_at_unix
                )
                SELECT DISTINCT ON (refs.object_key)
                    refs.object_key,
                    'm0023_logical_run_sources',
                    runs.source#>>'{manifest,sha256}',
                    runs.source#>>'{manifest,git_oid}',
                    (runs.source#>>'{manifest,size_bytes}')::bigint,
                    0, 0, NULL, NULL, 0, 0
                FROM scope_runs runs
                JOIN scope_object_references refs
                  ON refs.ref_kind = 'run_source'
                 AND refs.ref_id = runs.id
                 AND refs.object_key::jsonb = runs.source#>'{manifest,content_ref}'
                WHERE runs.source->>'kind' = 'accepted-revision'
                  AND NOT EXISTS (
                      SELECT 1
                      FROM scope_object_references other_refs
                      WHERE other_refs.object_key = refs.object_key
                        AND NOT (
                            other_refs.ref_kind = 'run_source'
                            AND EXISTS (
                                SELECT 1
                                FROM scope_runs other_runs
                                WHERE other_runs.id = other_refs.ref_id
                                  AND other_runs.source->>'kind' = 'accepted-revision'
                                  AND other_refs.object_key::jsonb =
                                      other_runs.source#>'{manifest,content_ref}'
                            )
                        )
                  )
                ORDER BY refs.object_key, runs.id
                ON CONFLICT (object_key) DO NOTHING;

                DELETE FROM scope_object_references refs
                USING scope_runs runs
                WHERE refs.ref_kind = 'run_source'
                  AND refs.ref_id = runs.id
                  AND runs.source->>'kind' = 'accepted-revision'
                  AND refs.object_key::jsonb = runs.source#>'{manifest,content_ref}';

                UPDATE scope_runs
                SET source = jsonb_build_object(
                    'kind', 'ephemeral-git-bundle',
                    'object', source->'snapshot'
                )
                WHERE source->>'kind' = 'accepted-revision';

                ALTER TABLE scope_runs
                    DROP CONSTRAINT scope_runs_values;
                ALTER TABLE scope_runs
                    ADD CONSTRAINT scope_runs_values CHECK (
                        char_length(workflow_revision_digest) = 64 AND
                        workflow_revision_digest ~ '^[0-9A-Fa-f]+$' AND
                        (
                            (
                                source->>'kind' = 'ephemeral-git-bundle' AND
                                char_length(source#>>'{object,sha256}') = 64 AND
                                (source#>>'{object,sha256}') ~ '^[0-9A-Fa-f]+$' AND
                                char_length(source#>>'{object,git_oid}') = 40 AND
                                (source#>>'{object,git_oid}') ~ '^[0-9A-Fa-f]+$'
                            ) OR (
                                source->>'kind' = 'accepted-git-head' AND
                                length(btrim(source->>'repository_id')) > 0 AND
                                source->>'audience' IN ('Private', 'Public') AND
                                (source#>>'{head,push_sequence}')::numeric > 0 AND
                                (source#>>'{head,change_version}')::numeric > 0 AND
                                char_length(source#>>'{head,head_oid}') = 40 AND
                                (source#>>'{head,head_oid}') ~ '^[0-9A-Fa-f]+$' AND
                                char_length(source#>>'{head,manifest,sha256}') = 64 AND
                                (source#>>'{head,manifest,sha256}') ~ '^[0-9A-Fa-f]+$' AND
                                char_length(source#>>'{head,manifest,git_oid}') = 40 AND
                                (source#>>'{head,manifest,git_oid}') ~ '^[0-9A-Fa-f]+$' AND
                                (source#>>'{head,manifest,git_oid}') = (source#>>'{head,head_oid}') AND
                                jsonb_typeof(source->'pack_spans') = 'array' AND
                                jsonb_array_length(source->'pack_spans') > 0 AND
                                ((source->'pack_spans')->(jsonb_array_length(source->'pack_spans') - 1)->>'last_sequence')::numeric =
                                    (source#>>'{head,push_sequence}')::numeric AND
                                ((source->'pack_spans')->(jsonb_array_length(source->'pack_spans') - 1)->>'head_oid') =
                                    (source#>>'{head,head_oid}')
                            )
                        ) AND
                        trigger IN ('manual', 'push-main') AND
                        state IN ('queued', 'dispatching', 'running', 'succeeded', 'failed', 'canceled', 'lost') AND
                        created_at_unix >= 0 AND updated_at_unix >= created_at_unix AND
                        ((state IN ('succeeded', 'failed', 'canceled', 'lost')) =
                            (completed_at_unix IS NOT NULL)) AND
                        (completed_at_unix IS NULL OR completed_at_unix = updated_at_unix) AND
                        (state <> 'canceled' OR cancellation_requested)
                    );

                ALTER TABLE scope_outbox_jobs
                    DROP CONSTRAINT scope_outbox_jobs_push_workflow_schema_v4;
                ALTER TABLE scope_outbox_jobs
                    ADD CONSTRAINT scope_outbox_jobs_push_workflow_schema_v5 CHECK (
                        kind <> 'push_main_trigger_evaluation' OR
                        completed_at_unix IS NOT NULL OR
                        payload @> '{"workflow_schema_version": 5}'::jsonb
                    );
                "#,
            )
            .await?;
        Ok(())
    }
}
