use sea_orm::{ConnectionTrait, DatabaseBackend, Statement};
use sea_orm_migration::{DbErr, MigrationName, MigrationTrait, SchemaManager};

pub struct Migration;

impl MigrationName for Migration {
    fn name(&self) -> &str {
        "m0018_truthful_run_log_truncation"
    }
}

#[sea_orm_migration::async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let db = manager.get_connection();
        db.execute_unprepared(
            "LOCK TABLE scope_run_attempts, scope_run_jobs, scope_runs,
                        scope_workflow_revisions, scope_runners,
                        scope_runner_protocol_cutover, scope_runner_protocol_canaries
             IN ACCESS EXCLUSIVE MODE",
        )
        .await?;
        require_drained_runtime(db).await?;
        db
            .execute_unprepared(
                "DO $$
                 BEGIN
                     IF EXISTS (
                         SELECT 1 FROM scope_run_attempts WHERE logs_truncated = TRUE
                     ) THEN
                         RAISE EXCEPTION
                             'cannot infer the first truncated step from the retired attempt-wide fact';
                     END IF;
                 END $$;
                 ALTER TABLE scope_run_attempts DROP COLUMN logs_truncated;
                 ALTER TABLE scope_run_attempts
                     ADD COLUMN first_truncated_step_index integer;
                 ALTER TABLE scope_run_attempts
                     ADD CONSTRAINT scope_run_attempts_truncated_step_nonnegative
                     CHECK (
                         first_truncated_step_index IS NULL
                         OR first_truncated_step_index >= 0
                     );
                 ALTER TABLE scope_runner_protocol_cutover
                     DROP CONSTRAINT scope_runner_protocol_cutover_values;
                 ALTER TABLE scope_runners
                     DROP CONSTRAINT scope_runners_v6_cutover;
                 WITH cutover_time AS (
                     SELECT extract(epoch FROM clock_timestamp())::bigint AS now_unix
                 )
                 UPDATE scope_run_jobs AS job
                 SET state = 'canceled',
                     updated_at_unix = GREATEST(job.updated_at_unix, cutover_time.now_unix),
                     completed_at_unix = GREATEST(job.updated_at_unix, cutover_time.now_unix)
                 FROM scope_runner_protocol_canaries AS canary, scope_runs AS run, cutover_time
                 WHERE canary.run_id = run.id
                   AND job.run_id = run.id
                   AND run.state = 'queued'
                   AND job.state IN ('blocked', 'queued');
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
                   AND run.state = 'queued';
                 DELETE FROM scope_runner_protocol_canaries;
                 UPDATE scope_runner_protocol_cutover
                 SET state = CASE
                         WHEN EXISTS (SELECT 1 FROM scope_workflow_revisions)
                         THEN 'v7-fenced'
                         ELSE 'v7-open'
                     END,
                     canary_generation = 0,
                     updated_at_unix = extract(epoch FROM clock_timestamp())::bigint
                 WHERE key = 'current';
                 ALTER TABLE scope_runner_protocol_cutover
                     ADD CONSTRAINT scope_runner_protocol_cutover_values CHECK (
                         state IN ('v7-fenced', 'v7-open') AND
                         canary_generation >= 0 AND
                         updated_at_unix >= 0
                     );
                 UPDATE scope_runners SET enabled = FALSE WHERE protocol_version < 7;
                 ALTER TABLE scope_runners
                     ADD CONSTRAINT scope_runners_v7_cutover CHECK (
                         protocol_version >= 7 OR NOT enabled
                     );",
            )
            .await?;
        Ok(())
    }
}

async fn require_drained_runtime<C>(db: &C) -> Result<(), DbErr>
where
    C: ConnectionTrait,
{
    let count = db
        .query_one(Statement::from_string(
            DatabaseBackend::Postgres,
            "SELECT count(*) AS count FROM scope_run_attempts
             WHERE state IN ('leased', 'running')"
                .to_string(),
        ))
        .await?
        .ok_or_else(|| DbErr::Migration("PostgreSQL did not report active attempts".to_string()))?
        .try_get::<i64>("", "count")?;
    if count != 0 {
        return Err(DbErr::Migration(format!(
            "truthful run log truncation migration requires the runtime to drain; found {count} active run attempt(s)"
        )));
    }
    Ok(())
}
