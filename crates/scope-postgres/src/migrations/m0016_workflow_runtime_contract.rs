use sea_orm::{ConnectionTrait, DatabaseBackend, Statement};
use sea_orm_migration::{DbErr, MigrationName, MigrationTrait, SchemaManager};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;

const WORKFLOW_DIGEST_VERSION: u8 = 4;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PersistedWorkflowV3 {
    name: String,
    triggers: PersistedWorkflowTriggers,
    jobs: Vec<PersistedWorkflowJobV3>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PersistedWorkflowJobV3 {
    id: String,
    needs: Vec<String>,
    runner: PersistedRunnerSelector,
    container: PersistedWorkflowContainer,
    timeout_seconds: u64,
    caches: Vec<String>,
    steps: Vec<PersistedWorkflowStep>,
}

#[derive(Serialize)]
struct PersistedWorkflowV4 {
    name: String,
    triggers: PersistedWorkflowTriggers,
    jobs: Vec<PersistedWorkflowJobV4>,
}

#[derive(Serialize)]
struct PersistedWorkflowJobV4 {
    id: String,
    needs: Vec<String>,
    runner: PersistedRunnerSelector,
    container: PersistedWorkflowContainer,
    timeout_seconds: u64,
    caches: Vec<PersistedWorkflowCache>,
    environment: BTreeMap<String, String>,
    steps: Vec<PersistedWorkflowStep>,
}

#[derive(Serialize)]
struct PersistedWorkflowCache {
    name: String,
    path: String,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct PersistedWorkflowTriggers {
    manual: bool,
    push_main: bool,
}

#[derive(Deserialize, Serialize)]
#[serde(
    deny_unknown_fields,
    tag = "kind",
    content = "name",
    rename_all = "kebab-case"
)]
enum PersistedRunnerSelector {
    Any,
    Named(String),
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct PersistedWorkflowContainer {
    image: String,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct PersistedWorkflowStep {
    name: String,
    run: String,
}

pub struct Migration;

impl MigrationName for Migration {
    fn name(&self) -> &str {
        "m0016_workflow_runtime_contract"
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
        rewrite_workflow_revisions(db).await?;
        // Populated databases deliberately enter the fence without carrying V5
        // assignments forward. The fenced enqueue guard admits only canonical
        // V6 canary runs, which supplies the post-migration bootstrap path.
        db.execute_unprepared(
            "ALTER TABLE scope_outbox_jobs
                 DROP CONSTRAINT scope_outbox_jobs_push_workflow_schema_v3;
             ALTER TABLE scope_outbox_jobs
                 ADD CONSTRAINT scope_outbox_jobs_push_workflow_schema_v4 CHECK (
                     kind <> 'push_main_trigger_evaluation' OR
                     completed_at_unix IS NOT NULL OR
                     payload @> '{\"workflow_schema_version\": 4}'::jsonb
                 );
             ALTER TABLE scope_runner_protocol_cutover
                 DROP CONSTRAINT scope_runner_protocol_cutover_values;
             ALTER TABLE scope_runners
                 DROP CONSTRAINT scope_runners_v5_cutover;
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
                     THEN 'v6-fenced'
                     ELSE 'v6-open'
                 END,
                 canary_generation = 0,
                 updated_at_unix = extract(epoch FROM clock_timestamp())::bigint
             WHERE key = 'current';
             ALTER TABLE scope_runner_protocol_cutover
                 ADD CONSTRAINT scope_runner_protocol_cutover_values CHECK (
                     state IN ('v6-fenced', 'v6-open') AND
                     canary_generation >= 0 AND
                     updated_at_unix >= 0
                 );
             UPDATE scope_runners SET enabled = FALSE WHERE protocol_version < 6;
             ALTER TABLE scope_runners
                 ADD CONSTRAINT scope_runners_v6_cutover CHECK (
                     protocol_version >= 6 OR NOT enabled
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
                "workflow runtime contract migration requires the runtime to drain; found {count} {noun}"
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
                "workflow runtime rewrite produced an invalid digest mapping".to_string(),
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
                "workflow runtime revision digest collision".to_string(),
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

fn rewrite_definition(definition: Value) -> Result<(Value, String), DbErr> {
    let persisted: PersistedWorkflowV3 =
        serde_json::from_value(definition).map_err(|error| DbErr::Migration(error.to_string()))?;
    let rewritten = PersistedWorkflowV4 {
        name: persisted.name,
        triggers: persisted.triggers,
        jobs: persisted
            .jobs
            .into_iter()
            .map(|job| PersistedWorkflowJobV4 {
                id: job.id,
                needs: job.needs,
                runner: job.runner,
                container: job.container,
                timeout_seconds: job.timeout_seconds,
                caches: job
                    .caches
                    .into_iter()
                    .map(|name| PersistedWorkflowCache {
                        path: format!("/scope/cache/{name}"),
                        name,
                    })
                    .collect(),
                environment: BTreeMap::new(),
                steps: job.steps,
            })
            .collect(),
    };
    #[derive(Serialize)]
    struct DigestInput<'a> {
        version: u8,
        definition: &'a PersistedWorkflowV4,
    }
    let bytes = serde_json::to_vec(&DigestInput {
        version: WORKFLOW_DIGEST_VERSION,
        definition: &rewritten,
    })
    .map_err(|error| DbErr::Migration(error.to_string()))?;
    let digest = hex::encode(Sha256::digest(bytes));
    let definition =
        serde_json::to_value(rewritten).map_err(|error| DbErr::Migration(error.to_string()))?;
    Ok((definition, digest))
}

#[cfg(test)]
mod tests {
    use super::*;
    use scope_domain::runs::workflow::{
        CompiledWorkflow, WorkflowIdentity, WorkflowPath, WorkflowRevision,
    };
    use serde_json::json;

    #[test]
    fn rewrite_adds_explicit_cache_paths_and_empty_environment() {
        let (definition, digest) = rewrite_definition(json!({
            "name": "Checks",
            "triggers": { "manual": true, "push_main": false },
            "jobs": [{
                "id": "checks",
                "needs": [],
                "runner": { "kind": "any" },
                "container": { "image": "rust:1.94" },
                "timeout_seconds": 60,
                "caches": ["cargo"],
                "steps": [{ "name": "Test", "run": "cargo test" }]
            }]
        }))
        .unwrap();

        assert_eq!(
            definition["jobs"][0]["caches"][0],
            json!({ "name": "cargo", "path": "/scope/cache/cargo" })
        );
        assert_eq!(definition["jobs"][0]["environment"], json!({}));
        assert_eq!(digest.len(), 64);
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
