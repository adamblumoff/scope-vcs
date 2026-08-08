use sea_orm::{ConnectionTrait, DatabaseBackend, Statement};
use sea_orm_migration::{DbErr, MigrationName, MigrationTrait, SchemaManager};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;

const WORKFLOW_JOBS_DIGEST_VERSION: u8 = 3;
const MIGRATED_JOB_ID: &str = "checks";
const PUSH_MAIN_TRIGGER_EVALUATION_JOB_KIND: &str = "push_main_trigger_evaluation";
const PUSH_MAIN_TRIGGER_WORKFLOW_SCHEMA_VERSION: u8 = 3;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PersistedWorkflowV4 {
    name: String,
    triggers: PersistedWorkflowTriggers,
    runner: PersistedRunnerSelector,
    container: PersistedWorkflowContainer,
    timeout_seconds: u64,
    caches: Vec<String>,
    steps: Vec<PersistedWorkflowStep>,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct PersistedWorkflowJobsV3 {
    name: String,
    triggers: PersistedWorkflowTriggers,
    jobs: Vec<PersistedWorkflowJobV3>,
}

#[derive(Deserialize, Serialize)]
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
        "m0013_workflow_jobs"
    }
}

#[sea_orm_migration::async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let db = manager.get_connection();
        db.execute_unprepared(
            "LOCK TABLE scope_outbox_jobs, scope_run_attempts, scope_runs, scope_workflow_revisions
             IN ACCESS EXCLUSIVE MODE",
        )
        .await?;
        let pending_push_trigger_evaluations = db
            .query_one(Statement::from_string(
                DatabaseBackend::Postgres,
                format!(
                    "SELECT count(*) AS count
                     FROM scope_outbox_jobs
                     WHERE kind = '{PUSH_MAIN_TRIGGER_EVALUATION_JOB_KIND}'
                       AND completed_at_unix IS NULL"
                ),
            ))
            .await?
            .ok_or_else(|| {
                DbErr::Migration(
                    "PostgreSQL did not report pending push trigger evaluations".to_string(),
                )
            })?
            .try_get::<i64>("", "count")?;
        if pending_push_trigger_evaluations != 0 {
            return Err(DbErr::Migration(format!(
                "workflow jobs migration requires all pending push trigger evaluations to finish before deployment; found {pending_push_trigger_evaluations} unfinished evaluation(s)"
            )));
        }
        let active_attempts = db
            .query_one(Statement::from_string(
                DatabaseBackend::Postgres,
                "SELECT count(*) AS count FROM scope_run_attempts WHERE state IN ('leased', 'running')"
                    .to_string(),
            ))
            .await?
            .ok_or_else(|| DbErr::Migration("PostgreSQL did not report active attempts".to_string()))?
            .try_get::<i64>("", "count")?;
        if active_attempts != 0 {
            return Err(DbErr::Migration(format!(
                "workflow jobs migration requires all run attempts to drain before deployment; found {active_attempts} active attempt(s)"
            )));
        }

        db.execute_unprepared(
            "ALTER TABLE scope_workflow_revisions
                 DROP CONSTRAINT scope_workflow_revisions_v4_cutover;",
        )
        .await?;
        rewrite_workflow_revisions(db).await?;
        db.execute_unprepared(
            "ALTER TABLE scope_workflow_revisions
                 ADD CONSTRAINT scope_workflow_revisions_jobs_shape CHECK (
                     jsonb_typeof(definition) = 'object' AND
                     definition ? 'jobs' AND
                     jsonb_typeof(definition -> 'jobs') = 'array' AND
                     jsonb_array_length(definition -> 'jobs') BETWEEN 1 AND 64 AND
                     NOT (definition ?| ARRAY[
                         'runner', 'container', 'timeout_seconds', 'caches', 'steps'
                     ])
                 );",
        )
        .await?;
        db.execute_unprepared(&format!(
            "ALTER TABLE scope_outbox_jobs
                 ADD CONSTRAINT scope_outbox_jobs_push_workflow_schema_v3 CHECK (
                     kind <> '{PUSH_MAIN_TRIGGER_EVALUATION_JOB_KIND}' OR
                     completed_at_unix IS NOT NULL OR
                     payload @> '{{\"workflow_schema_version\": {PUSH_MAIN_TRIGGER_WORKFLOW_SCHEMA_VERSION}}}'::jsonb
                 );"
        ))
        .await?;
        Ok(())
    }
}

async fn rewrite_workflow_revisions<C>(db: &C) -> Result<(), DbErr>
where
    C: ConnectionTrait,
{
    let rows = db
        .query_all(Statement::from_string(
            DatabaseBackend::Postgres,
            "SELECT digest, definition, created_at_unix
             FROM scope_workflow_revisions
             ORDER BY digest
             FOR UPDATE"
                .to_string(),
        ))
        .await?;
    let mut rewrites = BTreeMap::new();
    for row in rows {
        let old_digest = row.try_get::<String>("", "digest")?;
        let definition = row.try_get::<Value>("", "definition")?;
        let (definition, new_digest) = rewrite_definition(definition)?;
        if old_digest == new_digest {
            return Err(DbErr::Migration(
                "workflow jobs rewrite did not change the revision digest".to_string(),
            ));
        }
        if rewrites
            .insert(old_digest.clone(), new_digest.clone())
            .is_some()
        {
            return Err(DbErr::Migration(
                "duplicate persisted workflow revision digest".to_string(),
            ));
        }
        db.execute(Statement::from_sql_and_values(
            DatabaseBackend::Postgres,
            "INSERT INTO scope_workflow_revisions (digest, definition, created_at_unix)
             VALUES ($1, $2, $3)
             ON CONFLICT (digest) DO NOTHING",
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
            .ok_or_else(|| {
                DbErr::Migration("rewritten workflow revision was not persisted".to_string())
            })?
            .try_get::<Value>("", "definition")?;
        if persisted != definition {
            return Err(DbErr::Migration(
                "workflow revision digest collision during jobs rewrite".to_string(),
            ));
        }
    }
    for (old_digest, new_digest) in rewrites {
        db.execute(Statement::from_sql_and_values(
            DatabaseBackend::Postgres,
            "UPDATE scope_runs
             SET workflow_revision_digest = $1
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
    let workflow: PersistedWorkflowV4 =
        serde_json::from_value(definition).map_err(|error| DbErr::Migration(error.to_string()))?;
    let workflow = PersistedWorkflowJobsV3 {
        name: workflow.name,
        triggers: workflow.triggers,
        jobs: vec![PersistedWorkflowJobV3 {
            id: MIGRATED_JOB_ID.to_string(),
            needs: Vec::new(),
            runner: workflow.runner,
            container: workflow.container,
            timeout_seconds: workflow.timeout_seconds,
            caches: workflow.caches,
            steps: workflow.steps,
        }],
    };
    let digest = workflow_jobs_v3_digest(&workflow)?;
    let definition =
        serde_json::to_value(workflow).map_err(|error| DbErr::Migration(error.to_string()))?;
    Ok((definition, digest))
}

fn workflow_jobs_v3_digest(workflow: &PersistedWorkflowJobsV3) -> Result<String, DbErr> {
    #[derive(Serialize)]
    struct DigestInput<'a> {
        version: u8,
        definition: &'a PersistedWorkflowJobsV3,
    }

    let bytes = serde_json::to_vec(&DigestInput {
        version: WORKFLOW_JOBS_DIGEST_VERSION,
        definition: workflow,
    })
    .map_err(|error| DbErr::Migration(error.to_string()))?;
    Ok(hex::encode(Sha256::digest(bytes)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn rewrite_freezes_the_jobs_v3_shape_and_digest() {
        let (definition, digest) = rewrite_definition(json!({
            "name": "Legacy",
            "triggers": { "manual": true, "push_main": false },
            "runner": { "kind": "any" },
            "container": { "image": "rust:1.90" },
            "timeout_seconds": 1200,
            "caches": [],
            "steps": [{ "name": "Test", "run": "cargo test" }]
        }))
        .unwrap();

        assert_eq!(
            definition,
            json!({
                "name": "Legacy",
                "triggers": { "manual": true, "push_main": false },
                "jobs": [{
                    "id": "checks",
                    "needs": [],
                    "runner": { "kind": "any" },
                    "container": { "image": "rust:1.90" },
                    "timeout_seconds": 1200,
                    "caches": [],
                    "steps": [{ "name": "Test", "run": "cargo test" }]
                }]
            })
        );
        assert_eq!(
            digest,
            "c1a831feffae11e2325937e5121f70cee2f0fb826d23cc800960031c2aab3bc0"
        );
    }
}
