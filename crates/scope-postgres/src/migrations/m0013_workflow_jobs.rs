use scope_domain::runs::workflow::{
    CompiledWorkflow, WorkflowIdentity, WorkflowPath, WorkflowRevision,
};
use sea_orm::{ConnectionTrait, DatabaseBackend, Statement};
use sea_orm_migration::{DbErr, MigrationName, MigrationTrait, SchemaManager};
use serde_json::{Map, Value, json};
use std::collections::BTreeMap;

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
            "LOCK TABLE scope_run_attempts, scope_runs, scope_workflow_revisions
             IN ACCESS EXCLUSIVE MODE",
        )
        .await?;
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
    let identity = WorkflowIdentity::new(
        "workflow-jobs-migration",
        WorkflowPath::parse("/.scope/runs/migration.yml")
            .map_err(|error| DbErr::Migration(error.to_string()))?,
    )
    .map_err(|error| DbErr::Migration(error.to_string()))?;
    let mut rewrites = BTreeMap::new();
    for row in rows {
        let old_digest = row.try_get::<String>("", "digest")?;
        let definition = row.try_get::<Value>("", "definition")?;
        let definition = rewrite_definition(definition)?;
        let compiled: CompiledWorkflow = serde_json::from_value(definition.clone())
            .map_err(|error| DbErr::Migration(error.to_string()))?;
        let revision = WorkflowRevision::new(identity.clone(), compiled)
            .map_err(|error| DbErr::Migration(error.to_string()))?;
        let new_digest = revision.digest().to_string();
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

fn rewrite_definition(definition: Value) -> Result<Value, DbErr> {
    let mut workflow = definition.as_object().cloned().ok_or_else(|| {
        DbErr::Migration("persisted workflow definition must be an object".to_string())
    })?;
    if workflow.contains_key("jobs") {
        return Err(DbErr::Migration(
            "workflow jobs rewrite encountered an already-migrated definition".to_string(),
        ));
    }
    let job = json!({
        "id": "checks",
        "needs": [],
        "runner": take_required(&mut workflow, "runner")?,
        "container": take_required(&mut workflow, "container")?,
        "timeout_seconds": take_required(&mut workflow, "timeout_seconds")?,
        "caches": take_required(&mut workflow, "caches")?,
        "steps": take_required(&mut workflow, "steps")?,
    });
    workflow.insert("jobs".to_string(), Value::Array(vec![job]));
    Ok(Value::Object(workflow))
}

fn take_required(workflow: &mut Map<String, Value>, field: &str) -> Result<Value, DbErr> {
    workflow.remove(field).ok_or_else(|| {
        DbErr::Migration(format!(
            "persisted workflow definition is missing {field:?}"
        ))
    })
}
