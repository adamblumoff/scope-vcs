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
                LOCK TABLE scope_runs, scope_outbox_jobs, scope_object_references,
                    scope_orphan_object_jobs IN ACCESS EXCLUSIVE MODE;

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
