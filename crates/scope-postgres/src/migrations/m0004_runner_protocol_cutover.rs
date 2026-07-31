use scope_domain::runs::workflow::{
    CompiledWorkflow, WorkflowIdentity, WorkflowPath, WorkflowRevision,
};
use sea_orm::{ConnectionTrait, DatabaseBackend, Statement};
use sea_orm_migration::{DbErr, MigrationName, MigrationTrait, SchemaManager};
use serde_json::Value;
use std::collections::BTreeMap;

pub struct Migration;

impl MigrationName for Migration {
    fn name(&self) -> &str {
        "m0004_runner_protocol_cutover"
    }
}

#[sea_orm_migration::async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let db = manager.get_connection();
        db.execute_unprepared(
            "LOCK TABLE scope_run_attempts, scope_runs, scope_workflow_revisions, scope_runners
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
                "protocol V4 cutover requires all V3 attempts to drain before deployment; found {active_attempts} active attempt(s)"
            )));
        }
        rewrite_workflow_revisions_to_v4(db).await?;

        db.execute_unprepared(
                "
                    UPDATE scope_runners
                    SET enabled = FALSE
                    WHERE protocol_version < 4;

                    ALTER TABLE scope_runners
                        ADD CONSTRAINT scope_runners_v4_cutover CHECK (
                            protocol_version >= 4 OR NOT enabled
                        );

                    ALTER TABLE scope_workflow_revisions
                        ADD CONSTRAINT scope_workflow_revisions_v4_cutover CHECK (
                            definition ? 'caches' AND
                            jsonb_typeof(definition -> 'caches') = 'array'
                        );

                    CREATE TABLE scope_runner_protocol_cutover (
                        key character varying PRIMARY KEY,
                        state character varying NOT NULL,
                        canary_generation bigint NOT NULL,
                        updated_at_unix bigint NOT NULL,
                        CONSTRAINT scope_runner_protocol_cutover_singleton CHECK (key = 'current'),
                        CONSTRAINT scope_runner_protocol_cutover_values CHECK (
                            state IN ('v3-open', 'v3-draining', 'rewrite-v4', 'v4-fenced', 'v4-open') AND
                            canary_generation >= 0 AND
                            updated_at_unix >= 0
                        )
                    );

                    CREATE TABLE scope_runner_protocol_canaries (
                        generation bigint NOT NULL,
                        phase character varying NOT NULL,
                        runner_id character varying NOT NULL,
                        run_id character varying NOT NULL,
                        status character varying NOT NULL,
                        created_at_unix bigint NOT NULL,
                        updated_at_unix bigint NOT NULL,
                        PRIMARY KEY (generation, phase),
                        UNIQUE (run_id),
                        CONSTRAINT fk_scope_runner_protocol_canaries_runner
                            FOREIGN KEY (runner_id) REFERENCES scope_runners(id) ON DELETE RESTRICT,
                        CONSTRAINT fk_scope_runner_protocol_canaries_run
                            FOREIGN KEY (run_id) REFERENCES scope_runs(id) ON DELETE RESTRICT,
                        CONSTRAINT scope_runner_protocol_canaries_values CHECK (
                            generation > 0 AND
                            phase IN ('cold-write', 'warm-read', 'evict') AND
                            status IN ('pending', 'running', 'succeeded', 'failed') AND
                            created_at_unix >= 0 AND
                            updated_at_unix >= created_at_unix
                        )
                    );

                    CREATE INDEX idx_scope_runner_protocol_canaries_runner_status
                        ON scope_runner_protocol_canaries (runner_id, status);

                    INSERT INTO scope_runner_protocol_cutover (
                        key, state, canary_generation, updated_at_unix
                    )
                    SELECT
                        'current',
                        CASE
                            WHEN EXISTS (SELECT 1 FROM scope_workflow_revisions)
                            THEN 'v4-fenced'
                            ELSE 'v4-open'
                        END,
                        0,
                        extract(epoch FROM clock_timestamp())::bigint;
                ",
            )
            .await?;
        Ok(())
    }
}

async fn rewrite_workflow_revisions_to_v4<C>(db: &C) -> Result<(), DbErr>
where
    C: ConnectionTrait,
{
    let rows = db
        .query_all(Statement::from_string(
            DatabaseBackend::Postgres,
            "SELECT digest, definition, created_at_unix FROM scope_workflow_revisions ORDER BY digest FOR UPDATE"
                .to_string(),
        ))
        .await?;
    let identity = WorkflowIdentity::new(
        "cutover",
        WorkflowPath::parse("/.scope/runs/cutover.yml")
            .map_err(|error| DbErr::Migration(error.to_string()))?,
    )
    .map_err(|error| DbErr::Migration(error.to_string()))?;
    let mut rewrites = BTreeMap::new();
    for row in rows {
        let old_digest = row.try_get::<String>("", "digest")?;
        let mut definition = row.try_get::<Value>("", "definition")?;
        let object = definition.as_object_mut().ok_or_else(|| {
            DbErr::Migration("persisted workflow definition must be an object".to_string())
        })?;
        if object
            .insert("caches".to_string(), Value::Array(Vec::new()))
            .is_some()
        {
            return Err(DbErr::Migration(
                "workflow rewrite encountered an already-V4 definition".to_string(),
            ));
        }
        let compiled: CompiledWorkflow = serde_json::from_value(definition.clone())
            .map_err(|error| DbErr::Migration(error.to_string()))?;
        let revision = WorkflowRevision::new(identity.clone(), compiled)
            .map_err(|error| DbErr::Migration(error.to_string()))?;
        let new_digest = revision.digest().to_string();
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
             VALUES ($1, $2, $3)",
            [
                new_digest.into(),
                definition.into(),
                row.try_get::<i64>("", "created_at_unix")?.into(),
            ],
        ))
        .await?;
    }
    for (old_digest, new_digest) in rewrites {
        db.execute(Statement::from_sql_and_values(
            DatabaseBackend::Postgres,
            "UPDATE scope_runs SET workflow_revision_digest = $1 WHERE workflow_revision_digest = $2",
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
