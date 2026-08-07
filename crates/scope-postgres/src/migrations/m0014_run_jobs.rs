use sea_orm::{ConnectionTrait, DatabaseBackend, Statement};
use sea_orm_migration::{DbErr, MigrationName, MigrationTrait, SchemaManager};

pub struct Migration;

impl MigrationName for Migration {
    fn name(&self) -> &str {
        "m0014_run_jobs"
    }
}

#[sea_orm_migration::async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let db = manager.get_connection();
        db.execute_unprepared(
            "LOCK TABLE scope_runs, scope_run_attempts, scope_workflow_revisions,
                        scope_runner_protocol_cutover, scope_runner_protocol_canaries
             IN ACCESS EXCLUSIVE MODE",
        )
        .await?;
        let active_attempts = db
            .query_one(Statement::from_string(
                DatabaseBackend::Postgres,
                "SELECT count(*) AS count FROM scope_run_attempts
                 WHERE state IN ('leased', 'running')"
                    .to_string(),
            ))
            .await?
            .ok_or_else(|| DbErr::Migration("PostgreSQL did not report active attempts".into()))?
            .try_get::<i64>("", "count")?;
        if active_attempts != 0 {
            return Err(DbErr::Migration(format!(
                "run-job cutover requires active attempts to drain; found {active_attempts}"
            )));
        }
        let multi_job_runs = db
            .query_one(Statement::from_string(
                DatabaseBackend::Postgres,
                "SELECT count(*) AS count
                 FROM scope_runs run
                 JOIN scope_workflow_revisions revision
                   ON revision.digest = run.workflow_revision_digest
                 WHERE revision.definition ? 'jobs'
                   AND jsonb_array_length(revision.definition -> 'jobs') <> 1"
                    .to_string(),
            ))
            .await?
            .ok_or_else(|| DbErr::Migration("PostgreSQL did not report workflow jobs".into()))?
            .try_get::<i64>("", "count")?;
        if multi_job_runs != 0 {
            return Err(DbErr::Migration(format!(
                "run-job cutover cannot infer history for {multi_job_runs} multi-job run(s)"
            )));
        }

        db.execute_unprepared(
            r#"
                ALTER TABLE scope_runs DROP CONSTRAINT fk_scope_runs_current_attempt;
                ALTER TABLE scope_runs DROP CONSTRAINT scope_runs_values;
                ALTER TABLE scope_runners DROP CONSTRAINT scope_runners_v4_cutover;
                ALTER TABLE scope_runner_protocol_cutover
                    DROP CONSTRAINT scope_runner_protocol_cutover_values;
                WITH cutover_time AS (
                    SELECT extract(epoch FROM clock_timestamp())::bigint AS now_unix
                )
                UPDATE scope_runs AS run
                SET state = 'canceled',
                    cancellation_requested = TRUE,
                    updated_at_unix = GREATEST(run.updated_at_unix, cutover_time.now_unix),
                    completed_at_unix = GREATEST(run.updated_at_unix, cutover_time.now_unix)
                FROM scope_runner_protocol_canaries AS canary, cutover_time
                WHERE canary.run_id = run.id
                  AND run.state = 'queued'
                  AND run.current_attempt_id IS NULL;
                DELETE FROM scope_runner_protocol_canaries;
                UPDATE scope_runner_protocol_cutover
                SET state = CASE
                        WHEN EXISTS (SELECT 1 FROM scope_workflow_revisions)
                        THEN 'v5-fenced'
                        ELSE 'v5-open'
                    END,
                    canary_generation = 0,
                    updated_at_unix = extract(epoch FROM clock_timestamp())::bigint
                WHERE key = 'current';
                ALTER TABLE scope_runner_protocol_cutover
                    ADD CONSTRAINT scope_runner_protocol_cutover_values CHECK (
                        state IN ('v5-fenced', 'v5-open') AND
                        canary_generation >= 0 AND
                        updated_at_unix >= 0
                    );
                UPDATE scope_runners SET enabled = FALSE WHERE protocol_version < 5;
                ALTER TABLE scope_runners
                    ADD CONSTRAINT scope_runners_v5_cutover CHECK (
                        protocol_version >= 5 OR NOT enabled
                    );

                CREATE TABLE scope_run_jobs (
                    run_id character varying NOT NULL,
                    job_key character varying NOT NULL,
                    desired_runner_name character varying,
                    pinned_container_image text,
                    state character varying NOT NULL,
                    last_attempt_number integer NOT NULL,
                    current_attempt_id character varying,
                    created_at_unix bigint NOT NULL,
                    updated_at_unix bigint NOT NULL,
                    completed_at_unix bigint,
                    PRIMARY KEY (run_id, job_key),
                    CONSTRAINT fk_scope_run_jobs_run
                        FOREIGN KEY (run_id) REFERENCES scope_runs(id) ON DELETE CASCADE,
                    CONSTRAINT scope_run_jobs_values CHECK (
                        char_length(job_key) BETWEEN 1 AND 64 AND
                        job_key ~ '^[a-z0-9]+(-[a-z0-9]+)*$' AND
                        (desired_runner_name IS NULL OR
                            char_length(desired_runner_name) BETWEEN 1 AND 64) AND
                        (pinned_container_image IS NULL OR
                            pinned_container_image ~ '^[^@[:space:]]+@sha256:[0-9A-Fa-f]{64}$') AND
                        state IN ('blocked', 'queued', 'leased', 'running', 'succeeded',
                                  'failed', 'skipped', 'canceled', 'lost') AND
                        last_attempt_number >= 0 AND
                        created_at_unix >= 0 AND updated_at_unix >= created_at_unix AND
                        ((state IN ('leased', 'running')) = (current_attempt_id IS NOT NULL)) AND
                        ((state IN ('succeeded', 'failed', 'skipped', 'canceled', 'lost')) =
                            (completed_at_unix IS NOT NULL)) AND
                        (completed_at_unix IS NULL OR completed_at_unix = updated_at_unix) AND
                        (state NOT IN ('running', 'succeeded') OR pinned_container_image IS NOT NULL)
                    )
                );

                INSERT INTO scope_run_jobs (
                    run_id, job_key, desired_runner_name, pinned_container_image, state,
                    last_attempt_number, current_attempt_id, created_at_unix,
                    updated_at_unix, completed_at_unix
                )
                SELECT run.id,
                       revision.definition #>> '{jobs,0,id}',
                       run.desired_runner_name, run.pinned_container_image, run.state,
                       run.last_attempt_number,
                       CASE WHEN run.state IN ('leased', 'running')
                            THEN run.current_attempt_id
                            ELSE NULL
                       END,
                       run.created_at_unix,
                       run.updated_at_unix, run.completed_at_unix
                FROM scope_runs run
                JOIN scope_workflow_revisions revision
                  ON revision.digest = run.workflow_revision_digest;

                DROP INDEX idx_scope_run_attempts_active;
                ALTER TABLE scope_run_attempts
                    DROP CONSTRAINT scope_run_attempts_run_id_number_key,
                    ADD COLUMN job_key character varying;
                UPDATE scope_run_attempts attempt
                SET job_key = job.job_key
                FROM scope_run_jobs job
                WHERE job.run_id = attempt.run_id;
                ALTER TABLE scope_run_attempts
                    ALTER COLUMN job_key SET NOT NULL,
                    ADD CONSTRAINT scope_run_attempts_run_id_job_key_number_key
                        UNIQUE (run_id, job_key, number),
                    ADD CONSTRAINT scope_run_attempts_identity_key
                        UNIQUE (id, run_id, job_key),
                    ADD CONSTRAINT fk_scope_run_attempts_job
                        FOREIGN KEY (run_id, job_key)
                        REFERENCES scope_run_jobs(run_id, job_key) ON DELETE CASCADE;

                ALTER TABLE scope_run_jobs
                    ADD CONSTRAINT fk_scope_run_jobs_current_attempt
                    FOREIGN KEY (current_attempt_id, run_id, job_key)
                    REFERENCES scope_run_attempts(id, run_id, job_key)
                    ON DELETE SET NULL (current_attempt_id);

                CREATE UNIQUE INDEX idx_scope_run_attempts_active
                    ON scope_run_attempts (run_id, job_key)
                    WHERE state IN ('leased', 'running');
                CREATE INDEX idx_scope_run_jobs_dispatch
                    ON scope_run_jobs (desired_runner_name, created_at_unix, run_id, job_key)
                    WHERE state = 'queued';

                ALTER TABLE scope_runs
                    ADD COLUMN runner_override_name character varying;
                UPDATE scope_runs AS run
                SET runner_override_name = run.desired_runner_name
                FROM scope_workflow_revisions AS revision
                WHERE revision.digest = run.workflow_revision_digest
                  AND run.trigger = 'manual'
                  AND run.desired_runner_name IS NOT NULL
                  AND run.desired_runner_name IS DISTINCT FROM
                      (revision.definition #>> '{jobs,0,runner,name}');
                ALTER TABLE scope_runs
                    DROP COLUMN pinned_container_image,
                    DROP COLUMN desired_runner_name,
                    DROP COLUMN last_attempt_number,
                    DROP COLUMN current_attempt_id,
                    ADD CONSTRAINT scope_runs_values CHECK (
                        char_length(workflow_revision_digest) = 64 AND
                        workflow_revision_digest ~ '^[0-9A-Fa-f]+$' AND
                        trigger IN ('manual', 'push-main') AND
                        state IN ('queued', 'leased', 'running', 'succeeded', 'failed',
                                  'canceled', 'lost') AND
                        created_at_unix >= 0 AND updated_at_unix >= created_at_unix AND
                        ((state IN ('succeeded', 'failed', 'canceled', 'lost')) =
                            (completed_at_unix IS NOT NULL)) AND
                        (completed_at_unix IS NULL OR completed_at_unix = updated_at_unix) AND
                        (state <> 'canceled' OR cancellation_requested)
                    );
            "#,
        )
        .await?;
        Ok(())
    }
}
