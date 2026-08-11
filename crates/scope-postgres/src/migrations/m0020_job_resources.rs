use scope_domain::runs::workflow::{
    CompiledWorkflow, WorkflowIdentity, WorkflowPath, WorkflowRevision,
};
use sea_orm::{ConnectionTrait, DatabaseBackend, Statement};
use sea_orm_migration::{DbErr, MigrationName, MigrationTrait, SchemaManager};
use serde_json::{Value, json};
use std::collections::BTreeMap;

const DEFAULT_CPU_MILLIS: u64 = 1_000;
const DEFAULT_MEMORY_BYTES: u64 = 1024 * 1024 * 1024;

pub struct Migration;

impl MigrationName for Migration {
    fn name(&self) -> &str {
        "m0020_job_resources"
    }
}

#[sea_orm_migration::async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let db = manager.get_connection();
        db.execute_unprepared(
            "LOCK TABLE scope_outbox_jobs, scope_run_attempts, scope_run_jobs, scope_runs,
                        scope_workflow_revisions, scope_runners,
                        scope_runner_protocol_cutover, scope_runner_protocol_canaries
             IN ACCESS EXCLUSIVE MODE",
        )
        .await?;
        require_drained_runtime(db).await?;
        db.execute_unprepared(
            "ALTER TABLE scope_run_jobs ADD COLUMN cpu_millis bigint;
             ALTER TABLE scope_run_jobs ADD COLUMN memory_bytes bigint;",
        )
        .await?;
        rewrite_workflow_revisions(db).await?;
        db.execute_unprepared(
            "UPDATE scope_run_jobs AS job
             SET cpu_millis = (definition_job.value -> 'resources' ->> 'cpu_millis')::bigint,
                 memory_bytes = (definition_job.value -> 'resources' ->> 'memory_bytes')::bigint
             FROM scope_runs AS run
             JOIN scope_workflow_revisions AS revision
               ON revision.digest = run.workflow_revision_digest
             CROSS JOIN LATERAL jsonb_array_elements(revision.definition -> 'jobs') AS definition_job(value)
             WHERE job.run_id = run.id
               AND definition_job.value ->> 'id' = job.job_key;
             ALTER TABLE scope_run_jobs ALTER COLUMN cpu_millis SET NOT NULL;
             ALTER TABLE scope_run_jobs ALTER COLUMN memory_bytes SET NOT NULL;
             ALTER TABLE scope_run_jobs
                 ADD CONSTRAINT scope_run_jobs_resources CHECK (
                     cpu_millis BETWEEN 500 AND 64000
                     AND memory_bytes BETWEEN 536870912 AND 1099511627776
                 );
             ALTER TABLE scope_outbox_jobs
                 DROP CONSTRAINT scope_outbox_jobs_push_workflow_schema_v4;
             ALTER TABLE scope_outbox_jobs
                 ADD CONSTRAINT scope_outbox_jobs_push_workflow_schema_v5 CHECK (
                     kind <> 'push_main_trigger_evaluation' OR
                     completed_at_unix IS NOT NULL OR
                     payload @> '{\"workflow_schema_version\": 5}'::jsonb
                 );
             ALTER TABLE scope_runner_protocol_cutover
                 DROP CONSTRAINT scope_runner_protocol_cutover_values;
             ALTER TABLE scope_runners
                 DROP CONSTRAINT scope_runners_v7_cutover;
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
                     THEN 'v8-fenced'
                     ELSE 'v8-open'
                 END,
                 canary_generation = 0,
                 updated_at_unix = extract(epoch FROM clock_timestamp())::bigint
             WHERE key = 'current';
             ALTER TABLE scope_runner_protocol_cutover
                 ADD CONSTRAINT scope_runner_protocol_cutover_values CHECK (
                     state IN ('v8-fenced', 'v8-open') AND
                     canary_generation >= 0 AND
                     updated_at_unix >= 0
                 );
             UPDATE scope_runners SET enabled = FALSE WHERE protocol_version < 8;
             ALTER TABLE scope_runners
                 ADD CONSTRAINT scope_runners_v8_cutover CHECK (
                     protocol_version >= 8 OR NOT enabled
                 );
             CREATE FUNCTION scope_notify_run_dispatch() RETURNS trigger
             LANGUAGE plpgsql AS $$
             BEGIN
                 IF NEW.state = 'queued'
                    AND (TG_OP = 'INSERT' OR OLD.state IS DISTINCT FROM NEW.state)
                 THEN
                     PERFORM pg_notify('scope_run_dispatch', '');
                 END IF;
                 RETURN NEW;
             END;
             $$;
             CREATE TRIGGER scope_run_jobs_dispatch_notify
             AFTER INSERT OR UPDATE OF state ON scope_run_jobs
             FOR EACH ROW EXECUTE FUNCTION scope_notify_run_dispatch();",
        )
        .await?;
        Ok(())
    }
}

async fn require_drained_runtime<C>(db: &C) -> Result<(), DbErr>
where
    C: ConnectionTrait,
{
    for (query, noun) in [
        (
            "SELECT count(*) AS count FROM scope_run_attempts WHERE state IN ('leased', 'running')",
            "active run attempt(s)",
        ),
        (
            "SELECT count(*) AS count FROM scope_outbox_jobs
             WHERE kind = 'push_main_trigger_evaluation' AND completed_at_unix IS NULL",
            "pending push trigger evaluation(s)",
        ),
    ] {
        let count = db
            .query_one(Statement::from_string(
                DatabaseBackend::Postgres,
                query.to_string(),
            ))
            .await?
            .ok_or_else(|| DbErr::Migration(format!("PostgreSQL did not report {noun}")))?
            .try_get::<i64>("", "count")?;
        if count != 0 {
            return Err(DbErr::Migration(format!(
                "job resource migration requires the runtime to drain; found {count} {noun}"
            )));
        }
    }
    Ok(())
}

async fn rewrite_workflow_revisions<C>(db: &C) -> Result<(), DbErr>
where
    C: ConnectionTrait,
{
    let rows = db
        .query_all(Statement::from_string(
            DatabaseBackend::Postgres,
            "SELECT digest, definition, created_at_unix
             FROM scope_workflow_revisions ORDER BY digest FOR UPDATE"
                .to_string(),
        ))
        .await?;
    let mut rewrites = BTreeMap::new();
    for row in rows {
        let old_digest = row.try_get::<String>("", "digest")?;
        let (definition, new_digest) = rewrite_definition(row.try_get("", "definition")?)?;
        if old_digest == new_digest
            || rewrites
                .insert(old_digest.clone(), new_digest.clone())
                .is_some()
        {
            return Err(DbErr::Migration(
                "job resource rewrite produced an invalid digest mapping".to_string(),
            ));
        }
        db.execute(Statement::from_sql_and_values(
            DatabaseBackend::Postgres,
            "INSERT INTO scope_workflow_revisions (digest, definition, created_at_unix)
             VALUES ($1, $2, $3) ON CONFLICT (digest) DO NOTHING",
            [
                new_digest.clone().into(),
                definition.clone().into(),
                row.try_get::<i64>("", "created_at_unix")?.into(),
            ],
        ))
        .await?;
        let persisted = db
            .query_one(Statement::from_sql_and_values(
                DatabaseBackend::Postgres,
                "SELECT definition FROM scope_workflow_revisions WHERE digest = $1",
                [new_digest.into()],
            ))
            .await?
            .ok_or_else(|| DbErr::Migration("rewritten workflow revision is missing".to_string()))?
            .try_get::<Value>("", "definition")?;
        if persisted != definition {
            return Err(DbErr::Migration(
                "job resource workflow revision digest collision".to_string(),
            ));
        }
    }
    for (old_digest, new_digest) in rewrites {
        db.execute(Statement::from_sql_and_values(
            DatabaseBackend::Postgres,
            "UPDATE scope_runs SET workflow_revision_digest = $1
             WHERE workflow_revision_digest = $2",
            [new_digest.into(), old_digest.clone().into()],
        ))
        .await?;
        db.execute(Statement::from_sql_and_values(
            DatabaseBackend::Postgres,
            "DELETE FROM scope_workflow_revisions WHERE digest = $1",
            [old_digest.into()],
        ))
        .await?;
    }
    Ok(())
}

fn rewrite_definition(mut definition: Value) -> Result<(Value, String), DbErr> {
    let jobs = definition
        .get_mut("jobs")
        .and_then(Value::as_array_mut)
        .ok_or_else(|| DbErr::Migration("workflow definition jobs are invalid".to_string()))?;
    for job in jobs {
        let job = job
            .as_object_mut()
            .ok_or_else(|| DbErr::Migration("workflow definition job is invalid".to_string()))?;
        if job.contains_key("resources") {
            return Err(DbErr::Migration(
                "workflow definition already contains job resources".to_string(),
            ));
        }
        job.insert(
            "resources".to_string(),
            json!({
                "cpu_millis": DEFAULT_CPU_MILLIS,
                "memory_bytes": DEFAULT_MEMORY_BYTES,
            }),
        );
    }
    let compiled: CompiledWorkflow =
        serde_json::from_value(definition).map_err(|error| DbErr::Migration(error.to_string()))?;
    let revision = WorkflowRevision::new(
        WorkflowIdentity::new(
            "migration",
            WorkflowPath::parse("/.scope/runs/migration.yml")
                .map_err(|error| DbErr::Migration(error.to_string()))?,
        )
        .map_err(|error| DbErr::Migration(error.to_string()))?,
        compiled,
    )
    .map_err(|error| DbErr::Migration(error.to_string()))?;
    let digest = revision.digest().to_string();
    let definition = serde_json::to_value(revision.definition())
        .map_err(|error| DbErr::Migration(error.to_string()))?;
    Ok((definition, digest))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rewrite_adds_explicit_resources_and_uses_the_domain_digest() {
        let (definition, digest) = rewrite_definition(json!({
            "name": "Checks",
            "triggers": { "manual": true, "push_main": false },
            "jobs": [{
                "id": "checks",
                "needs": [],
                "runner": { "kind": "any" },
                "container": { "image": "rust:1.94" },
                "timeout_seconds": 60,
                "caches": [],
                "environment": {},
                "steps": [{ "name": "Test", "run": "cargo test" }]
            }]
        }))
        .unwrap();

        assert_eq!(
            definition["jobs"][0]["resources"],
            json!({
                "cpu_millis": DEFAULT_CPU_MILLIS,
                "memory_bytes": DEFAULT_MEMORY_BYTES,
            })
        );
        let compiled: CompiledWorkflow = serde_json::from_value(definition).unwrap();
        let revision = WorkflowRevision::new(
            WorkflowIdentity::new(
                "repo-1",
                WorkflowPath::parse("/.scope/runs/checks.yml").unwrap(),
            )
            .unwrap(),
            compiled,
        )
        .unwrap();
        assert_eq!(revision.digest(), digest);
    }
}
