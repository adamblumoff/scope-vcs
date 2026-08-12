use sea_orm::ConnectionTrait;
use sea_orm_migration::{DbErr, MigrationName, MigrationTrait, SchemaManager};

pub struct Migration;

impl MigrationName for Migration {
    fn name(&self) -> &str {
        "m0020_cloud_execution"
    }
}

#[sea_orm_migration::async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .get_connection()
            .execute_unprepared(
                r#"
                LOCK TABLE scope_runs, scope_run_jobs, scope_run_attempts,
                    scope_workflow_revisions, scope_runner_protocol_cutover,
                    scope_runner_protocol_canaries, scope_runner_grants,
                    scope_runners IN ACCESS EXCLUSIVE MODE;

                DO $$
                BEGIN
                    IF EXISTS (
                        SELECT 1 FROM scope_run_attempts
                        WHERE state IN ('leased', 'running')
                    ) THEN
                        RAISE EXCEPTION 'cloud execution cutover requires zero active run attempts';
                    END IF;
                END $$;

                TRUNCATE TABLE scope_push_trigger_evaluations,
                    scope_run_attempt_caches, scope_run_logs, scope_run_attempt_steps,
                    scope_run_attempts, scope_run_jobs, scope_runs,
                    scope_workflow_revisions CASCADE;

                ALTER TABLE scope_runs
                    DROP CONSTRAINT scope_runs_values,
                    DROP COLUMN runner_override_name,
                    ADD CONSTRAINT scope_runs_values CHECK (
                        char_length(workflow_revision_digest) = 64 AND
                        workflow_revision_digest ~ '^[0-9A-Fa-f]+$' AND
                        trigger IN ('manual', 'push-main') AND
                        state IN ('queued', 'dispatching', 'running', 'succeeded', 'failed',
                                  'canceled', 'lost') AND
                        created_at_unix >= 0 AND updated_at_unix >= created_at_unix AND
                        ((state IN ('succeeded', 'failed', 'canceled', 'lost')) =
                            (completed_at_unix IS NOT NULL)) AND
                        (completed_at_unix IS NULL OR completed_at_unix = updated_at_unix) AND
                        (state <> 'canceled' OR cancellation_requested)
                    );

                DROP INDEX IF EXISTS idx_scope_run_jobs_dispatch;
                ALTER TABLE scope_run_jobs
                    DROP CONSTRAINT scope_run_jobs_values,
                    DROP COLUMN desired_runner_name,
                    ALTER COLUMN pinned_container_image SET NOT NULL,
                    ADD CONSTRAINT scope_run_jobs_values CHECK (
                        char_length(job_key) BETWEEN 1 AND 64 AND
                        job_key ~ '^[a-z0-9]+(-[a-z0-9]+)*$' AND
                        pinned_container_image ~ '^[^@[:space:]]+@sha256:[0-9A-Fa-f]{64}$' AND
                        state IN ('blocked', 'queued', 'dispatching', 'running', 'succeeded',
                                  'failed', 'skipped', 'canceled', 'lost') AND
                        last_attempt_number >= 0 AND
                        created_at_unix >= 0 AND updated_at_unix >= created_at_unix AND
                        ((state IN ('dispatching', 'running')) = (current_attempt_id IS NOT NULL)) AND
                        ((state IN ('succeeded', 'failed', 'skipped', 'canceled', 'lost')) =
                            (completed_at_unix IS NOT NULL)) AND
                        (completed_at_unix IS NULL OR completed_at_unix = updated_at_unix)
                    );
                CREATE INDEX idx_scope_run_jobs_dispatch
                    ON scope_run_jobs (created_at_unix, run_id, job_key)
                    WHERE state = 'queued';

                DROP INDEX IF EXISTS idx_scope_run_attempts_runner_state;
                ALTER TABLE scope_run_attempts
                    DROP CONSTRAINT fk_scope_run_attempts_runner,
                    DROP CONSTRAINT scope_run_attempts_values,
                    DROP COLUMN runner_id,
                    DROP COLUMN runner_name,
                    ADD COLUMN execution_provider varchar(32) NOT NULL DEFAULT 'northflank',
                    ADD COLUMN external_run_id text,
                    ADD COLUMN provider_abort_requested_at_unix bigint,
                    ADD COLUMN runtime_version text NOT NULL DEFAULT 'unassigned';
                ALTER TABLE scope_run_attempts
                    ALTER COLUMN execution_provider DROP DEFAULT,
                    ALTER COLUMN runtime_version DROP DEFAULT,
                    ADD CONSTRAINT scope_run_attempts_values CHECK (
                        number > 0 AND
                        execution_provider = 'northflank' AND
                        (external_run_id IS NULL OR char_length(external_run_id) > 0) AND
                        (provider_abort_requested_at_unix IS NULL OR provider_abort_requested_at_unix >= created_at_unix) AND
                        char_length(runtime_version) BETWEEN 1 AND 128 AND
                        char_length(token_hash) = 64 AND
                        token_hash ~ '^[0-9A-Fa-f]+$' AND
                        state IN ('dispatching', 'running', 'succeeded', 'failed', 'canceled', 'lost') AND
                        token_expires_at_unix = lease_expires_at_unix AND
                        created_at_unix >= 0 AND
                        last_heartbeat_at_unix >= created_at_unix AND
                        last_heartbeat_at_unix < lease_expires_at_unix AND
                        (started_at_unix IS NULL OR
                            (started_at_unix >= created_at_unix AND started_at_unix < lease_expires_at_unix)) AND
                        (completed_at_unix IS NULL OR completed_at_unix >= last_heartbeat_at_unix) AND
                        (started_at_unix IS NULL OR completed_at_unix IS NULL OR completed_at_unix >= started_at_unix) AND
                        log_bytes >= 0 AND log_bytes <= 10485760 AND
                        ((state IN ('succeeded', 'failed', 'canceled', 'lost')) = (completed_at_unix IS NOT NULL)) AND
                        (state <> 'succeeded' OR (started_at_unix IS NOT NULL AND terminal_reason IS NULL)) AND
                        (state NOT IN ('failed', 'canceled', 'lost') OR terminal_reason IS NOT NULL) AND
                        (state IN ('failed', 'canceled', 'lost') OR terminal_reason IS NULL)
                    );
                CREATE INDEX idx_scope_run_attempts_provider_state
                    ON scope_run_attempts (execution_provider, state, created_at_unix);
                CREATE UNIQUE INDEX idx_scope_run_attempts_external_run
                    ON scope_run_attempts (execution_provider, external_run_id)
                    WHERE external_run_id IS NOT NULL;

                CREATE TABLE scope_run_cache_objects (
                    identity_digest varchar(64) PRIMARY KEY,
                    object_key text NOT NULL UNIQUE,
                    checksum_sha256 varchar(64) NOT NULL,
                    size_bytes bigint NOT NULL,
                    generation bigint NOT NULL,
                    ready boolean NOT NULL,
                    updated_at_unix bigint NOT NULL,
                    CONSTRAINT scope_run_cache_objects_values CHECK (
                        identity_digest ~ '^[0-9a-f]{64}$' AND
                        checksum_sha256 ~ '^[0-9a-f]{64}$' AND
                        char_length(object_key) > 0 AND
                        size_bytes BETWEEN 0 AND 10737418240 AND
                        generation > 0 AND updated_at_unix >= 0
                    )
                );

                DROP TABLE scope_runner_protocol_canaries;
                DROP TABLE scope_runner_protocol_cutover;
                DROP TABLE scope_runner_grants;
                DROP TABLE scope_runners;
                "#,
            )
            .await?;
        Ok(())
    }
}
