use sea_orm::ConnectionTrait;
use sea_orm_migration::{DbErr, MigrationName, MigrationTrait, SchemaManager};

pub struct Migration;

impl MigrationName for Migration {
    fn name(&self) -> &str {
        "m0031_provider_neutral_run_attempts"
    }
}

#[sea_orm_migration::async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .get_connection()
            .execute_unprepared(
                "LOCK TABLE scope_run_attempts IN ACCESS EXCLUSIVE MODE;

                DO $$
                BEGIN
                    IF EXISTS (
                        SELECT 1
                        FROM scope_run_attempts
                        WHERE state NOT IN ('succeeded', 'failed', 'canceled', 'lost')
                    ) THEN
                        RAISE EXCEPTION 'm0031 requires every Northflank run attempt to be terminal before the Fargate cutover';
                    END IF;
                END
                $$;

                DROP INDEX idx_scope_run_attempts_provider_state;
                DROP INDEX idx_scope_run_attempts_external_run;

                ALTER TABLE scope_run_attempts
                    DROP CONSTRAINT scope_run_attempts_values;
                ALTER TABLE scope_run_attempts
                    RENAME COLUMN provider_abort_requested_at_unix TO runner_stop_claimed_at_unix;
                ALTER TABLE scope_run_attempts
                    ADD COLUMN runner_stop_completed_at_unix bigint;
                UPDATE scope_run_attempts
                    SET runner_stop_claimed_at_unix = created_at_unix,
                        runner_stop_completed_at_unix = created_at_unix
                    WHERE state IN ('succeeded', 'failed', 'canceled', 'lost')
                      AND runner_stop_completed_at_unix IS NULL;
                ALTER TABLE scope_run_attempts
                    DROP COLUMN execution_provider,
                    ADD CONSTRAINT scope_run_attempts_values CHECK (
                        number > 0 AND
                        (external_run_id IS NULL OR char_length(external_run_id) > 0) AND
                        (runner_stop_claimed_at_unix IS NULL OR runner_stop_claimed_at_unix >= created_at_unix) AND
                        (runner_stop_completed_at_unix IS NULL OR
                            (runner_stop_claimed_at_unix IS NOT NULL AND
                             runner_stop_completed_at_unix >= runner_stop_claimed_at_unix)) AND
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

                CREATE INDEX idx_scope_run_attempts_state
                    ON scope_run_attempts (state, created_at_unix);
                CREATE UNIQUE INDEX idx_scope_run_attempts_external_run
                    ON scope_run_attempts (external_run_id)
                    WHERE external_run_id IS NOT NULL;",
            )
            .await?;
        Ok(())
    }
}
