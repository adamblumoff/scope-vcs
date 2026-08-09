mod m0001_adopt_v6;
mod m0002_retire_reset_schema;
mod m0003_structured_run_attempts;
mod m0004_runner_protocol_cutover;
mod m0005_projection_head_oid;
mod m0006_drop_request_credits;
mod m0007_drop_review_ceremony;
mod m0008_one_way_request_submission;
mod m0009_request_ratings;
mod m0010_file_visibility_source_of_truth;
mod m0011_compact_request_started_events;
mod m0012_request_revisions;
mod m0013_workflow_jobs;
mod m0014_run_jobs;
mod m0015_runner_capacity;
mod m0016_workflow_runtime_contract;
mod m0017_run_history_indexes;

use sea_orm::{
    ConnectionTrait, DatabaseBackend, DatabaseConnection, DbErr, Statement, TransactionTrait,
};
use sea_orm_migration::{MigrationTrait, MigratorTrait};

const MIGRATION_LOCK: &str = "scope:metadata-migrations";
const MIGRATION_TABLE: &str = "seaql_migrations";

pub struct Migrator;

#[sea_orm_migration::async_trait::async_trait]
impl MigratorTrait for Migrator {
    fn migrations() -> Vec<Box<dyn MigrationTrait>> {
        vec![
            Box::new(m0001_adopt_v6::Migration),
            Box::new(m0002_retire_reset_schema::Migration),
            Box::new(m0003_structured_run_attempts::Migration),
            Box::new(m0004_runner_protocol_cutover::Migration),
            Box::new(m0005_projection_head_oid::Migration),
            Box::new(m0006_drop_request_credits::Migration),
            Box::new(m0007_drop_review_ceremony::Migration),
            Box::new(m0008_one_way_request_submission::Migration),
            Box::new(m0009_request_ratings::Migration),
            Box::new(m0010_file_visibility_source_of_truth::Migration),
            Box::new(m0011_compact_request_started_events::Migration),
            Box::new(m0012_request_revisions::Migration),
            Box::new(m0013_workflow_jobs::Migration),
            Box::new(m0014_run_jobs::Migration),
            Box::new(m0015_runner_capacity::Migration),
            Box::new(m0016_workflow_runtime_contract::Migration),
            Box::new(m0017_run_history_indexes::Migration),
        ]
    }
}

pub async fn apply(db: &DatabaseConnection) -> Result<(), DbErr> {
    let tx = db.begin().await?;
    tx.execute_unprepared(&format!(
        "SELECT pg_advisory_xact_lock(
            hashtextextended('{MIGRATION_LOCK}:' || current_schema(), 0)
        )"
    ))
    .await?;
    Migrator::up(&tx, None).await?;
    assert_exact_state(&tx).await?;
    tx.commit().await
}

pub async fn assert_exact_state<C>(db: &C) -> Result<(), DbErr>
where
    C: ConnectionTrait,
{
    let table_exists = db
        .query_one(Statement::from_sql_and_values(
            DatabaseBackend::Postgres,
            "SELECT to_regclass(
                format('%I.%I', current_schema(), $1)
            ) IS NOT NULL AS exists",
            [MIGRATION_TABLE.into()],
        ))
        .await?
        .ok_or_else(|| DbErr::Custom("PostgreSQL did not report migration state".to_string()))?
        .try_get::<bool>("", "exists")?;
    if !table_exists {
        return Err(DbErr::Custom(
            "Scope metadata migrations have not been applied".to_string(),
        ));
    }

    let actual = db
        .query_all(Statement::from_string(
            DatabaseBackend::Postgres,
            format!("SELECT version FROM {MIGRATION_TABLE} ORDER BY version"),
        ))
        .await?
        .into_iter()
        .map(|row| row.try_get::<String>("", "version"))
        .collect::<Result<Vec<_>, _>>()?;
    let expected = migration_names();
    if actual != expected {
        return Err(DbErr::Custom(format!(
            "Scope metadata migration state does not match this binary: expected [{}], found [{}]",
            expected.join(", "),
            actual.join(", ")
        )));
    }
    Ok(())
}

fn migration_names() -> Vec<String> {
    let mut names = Migrator::migrations()
        .into_iter()
        .map(|migration| migration.name().to_string())
        .collect::<Vec<_>>();
    names.sort();
    names
}
