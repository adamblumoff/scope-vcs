use sea_orm::ConnectionTrait;
use sea_orm_migration::{DbErr, MigrationName, MigrationTrait, SchemaManager};

pub struct Migration;

impl MigrationName for Migration {
    fn name(&self) -> &str {
        "m0003_structured_run_attempts"
    }
}

#[sea_orm_migration::async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .get_connection()
            .execute_unprepared(
                "
                    UPDATE scope_runs
                    SET state = 'queued',
                        cancellation_requested = FALSE,
                        last_attempt_number = 0,
                        current_attempt_id = NULL,
                        completed_at_unix = NULL;
                    ALTER TABLE scope_runs
                        DROP CONSTRAINT fk_scope_runs_current_attempt;
                    DROP TABLE scope_run_logs;
                    DROP TABLE scope_run_attempts;

                    CREATE TABLE scope_run_attempts (
                        id character varying PRIMARY KEY,
                        run_id character varying NOT NULL,
                        number integer NOT NULL,
                        runner_id character varying NOT NULL,
                        runner_name character varying NOT NULL,
                        token_hash character varying NOT NULL UNIQUE,
                        token_expires_at_unix bigint NOT NULL,
                        state character varying NOT NULL,
                        lease_expires_at_unix bigint NOT NULL,
                        last_heartbeat_at_unix bigint NOT NULL,
                        created_at_unix bigint NOT NULL,
                        started_at_unix bigint,
                        completed_at_unix bigint,
                        terminal_reason jsonb,
                        log_bytes bigint NOT NULL,
                        logs_truncated boolean NOT NULL,
                        UNIQUE (run_id, number),
                        CONSTRAINT fk_scope_run_attempts_run
                            FOREIGN KEY (run_id) REFERENCES scope_runs(id) ON DELETE CASCADE,
                        CONSTRAINT fk_scope_run_attempts_runner
                            FOREIGN KEY (runner_id) REFERENCES scope_runners(id) ON DELETE RESTRICT,
                        CONSTRAINT scope_run_attempts_values CHECK (
                            number > 0 AND
                            char_length(runner_name) BETWEEN 1 AND 64 AND
                            char_length(token_hash) = 64 AND
                            token_hash ~ '^[0-9A-Fa-f]+$' AND
                            state IN ('leased', 'running', 'succeeded', 'failed', 'canceled', 'lost') AND
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
                        )
                    );

                    CREATE TABLE scope_run_attempt_steps (
                        attempt_id character varying NOT NULL,
                        step_index integer NOT NULL,
                        state character varying NOT NULL,
                        started_at_unix bigint,
                        completed_at_unix bigint,
                        exit_code integer,
                        PRIMARY KEY (attempt_id, step_index),
                        CONSTRAINT fk_scope_run_attempt_steps_attempt
                            FOREIGN KEY (attempt_id) REFERENCES scope_run_attempts(id) ON DELETE CASCADE,
                        CONSTRAINT scope_run_attempt_steps_values CHECK (
                            step_index >= 0 AND
                            state IN ('pending', 'running', 'succeeded', 'failed', 'canceled', 'lost', 'skipped') AND
                            ((state IN ('succeeded', 'failed', 'canceled', 'lost', 'skipped')) = (completed_at_unix IS NOT NULL)) AND
                            (started_at_unix IS NULL OR completed_at_unix IS NULL OR completed_at_unix >= started_at_unix) AND
                            (state <> 'pending' OR (started_at_unix IS NULL AND completed_at_unix IS NULL AND exit_code IS NULL)) AND
                            (state <> 'running' OR (started_at_unix IS NOT NULL AND completed_at_unix IS NULL AND exit_code IS NULL)) AND
                            (state <> 'succeeded' OR (started_at_unix IS NOT NULL AND exit_code = 0)) AND
                            (state <> 'failed' OR (started_at_unix IS NOT NULL AND exit_code IS NOT NULL AND exit_code <> 0)) AND
                            (state IN ('failed', 'succeeded') OR exit_code IS NULL) AND
                            (state <> 'skipped' OR started_at_unix IS NULL) AND
                            (state NOT IN ('canceled', 'lost') OR started_at_unix IS NOT NULL)
                        )
                    );

                    CREATE TABLE scope_run_logs (
                        position bigint GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
                        run_id character varying NOT NULL,
                        attempt_id character varying NOT NULL,
                        step_index integer NOT NULL,
                        sequence bigint NOT NULL,
                        text text NOT NULL,
                        created_at_unix bigint NOT NULL,
                        UNIQUE (attempt_id, sequence),
                        CONSTRAINT fk_scope_run_logs_run
                            FOREIGN KEY (run_id) REFERENCES scope_runs(id) ON DELETE CASCADE,
                        CONSTRAINT fk_scope_run_logs_step
                            FOREIGN KEY (attempt_id, step_index)
                            REFERENCES scope_run_attempt_steps(attempt_id, step_index) ON DELETE CASCADE,
                        CONSTRAINT scope_run_logs_values CHECK (
                            position > 0 AND
                            sequence > 0 AND
                            octet_length(text) BETWEEN 1 AND 65536 AND
                            created_at_unix >= 0
                        )
                    );

                    CREATE UNIQUE INDEX idx_scope_run_attempts_active
                        ON scope_run_attempts (run_id)
                        WHERE state IN ('leased', 'running');
                    CREATE INDEX idx_scope_run_attempts_runner
                        ON scope_run_attempts (runner_id, state);
                    CREATE INDEX idx_scope_run_attempts_expiring
                        ON scope_run_attempts (lease_expires_at_unix, id)
                        WHERE state IN ('leased', 'running');
                    CREATE INDEX idx_scope_run_logs_run_position
                        ON scope_run_logs (run_id, position);
                    CREATE INDEX idx_scope_run_logs_step_position
                        ON scope_run_logs (attempt_id, step_index, position);

                    ALTER TABLE scope_runs
                        ADD CONSTRAINT fk_scope_runs_current_attempt
                        FOREIGN KEY (current_attempt_id)
                        REFERENCES scope_run_attempts(id) ON DELETE SET NULL;
                ",
            )
            .await?;
        Ok(())
    }
}
