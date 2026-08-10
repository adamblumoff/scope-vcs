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
mod m0018_truthful_run_log_truncation;

use sea_orm::{
    ConnectionTrait, DatabaseBackend, DatabaseConnection, DbErr, Statement, TransactionTrait,
};
use sea_orm_migration::{MigrationTrait, MigratorTrait};
use serde::Serialize;

const MIGRATION_LOCK: &str = "scope:metadata-migrations";
const MIGRATION_TABLE: &str = "seaql_migrations";

pub struct Migrator;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum MigrationImpact {
    Online,
    MaintenanceRequired,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct PendingMigration {
    pub name: String,
    pub impact: MigrationImpact,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct MigrationPlan {
    pub exact: bool,
    pub pending: Vec<PendingMigration>,
}

struct MigrationSpec {
    migration: Box<dyn MigrationTrait>,
    impact: MigrationImpact,
}

fn spec(migration: impl MigrationTrait + 'static, impact: MigrationImpact) -> MigrationSpec {
    MigrationSpec {
        migration: Box::new(migration),
        impact,
    }
}

fn inventory() -> Vec<MigrationSpec> {
    use MigrationImpact::{MaintenanceRequired, Online};

    vec![
        spec(m0001_adopt_v6::Migration, MaintenanceRequired),
        spec(m0002_retire_reset_schema::Migration, MaintenanceRequired),
        spec(
            m0003_structured_run_attempts::Migration,
            MaintenanceRequired,
        ),
        spec(
            m0004_runner_protocol_cutover::Migration,
            MaintenanceRequired,
        ),
        spec(m0005_projection_head_oid::Migration, MaintenanceRequired),
        spec(m0006_drop_request_credits::Migration, MaintenanceRequired),
        spec(m0007_drop_review_ceremony::Migration, MaintenanceRequired),
        spec(
            m0008_one_way_request_submission::Migration,
            MaintenanceRequired,
        ),
        spec(m0009_request_ratings::Migration, Online),
        spec(
            m0010_file_visibility_source_of_truth::Migration,
            MaintenanceRequired,
        ),
        spec(
            m0011_compact_request_started_events::Migration,
            MaintenanceRequired,
        ),
        spec(m0012_request_revisions::Migration, MaintenanceRequired),
        spec(m0013_workflow_jobs::Migration, MaintenanceRequired),
        spec(m0014_run_jobs::Migration, MaintenanceRequired),
        spec(m0015_runner_capacity::Migration, MaintenanceRequired),
        spec(
            m0016_workflow_runtime_contract::Migration,
            MaintenanceRequired,
        ),
        spec(m0017_run_history_indexes::Migration, Online),
        spec(
            m0018_truthful_run_log_truncation::Migration,
            MaintenanceRequired,
        ),
    ]
}

#[sea_orm_migration::async_trait::async_trait]
impl MigratorTrait for Migrator {
    fn migrations() -> Vec<Box<dyn MigrationTrait>> {
        inventory().into_iter().map(|spec| spec.migration).collect()
    }
}

pub async fn apply_in_maintenance(db: &DatabaseConnection) -> Result<(), DbErr> {
    let tx = db.begin().await?;
    lock_migration_inventory(&tx).await?;
    Migrator::up(&tx, None).await?;
    assert_exact_state(&tx).await?;
    tx.commit().await
}

pub async fn apply_online(db: &DatabaseConnection) -> Result<(), DbErr> {
    let tx = db.begin().await?;
    lock_migration_inventory(&tx).await?;
    let pending = plan(&tx).await?.pending;
    let online_count = pending
        .iter()
        .take_while(|migration| migration.impact == MigrationImpact::Online)
        .count();
    if online_count > 0 {
        Migrator::up(&tx, Some(online_count as u32)).await?;
    }
    tx.commit().await?;

    let remaining = plan(db).await?.pending;
    if let Some(blocked) = remaining.first() {
        return Err(DbErr::Custom(format!(
            "migration {} requires a maintenance cutover; ordinary startup will not apply it",
            blocked.name
        )));
    }
    Ok(())
}

pub async fn plan<C>(db: &C) -> Result<MigrationPlan, DbErr>
where
    C: ConnectionTrait,
{
    let actual = applied_migration_names(db).await?;
    let specs = inventory();
    let expected = specs
        .iter()
        .map(|spec| spec.migration.name().to_string())
        .collect::<Vec<_>>();
    if !expected.starts_with(&actual) {
        return Err(DbErr::Custom(format!(
            "Scope metadata migration ledger is not a canonical prefix: expected [{}], found [{}]",
            expected.join(", "),
            actual.join(", ")
        )));
    }
    let pending = specs
        .into_iter()
        .skip(actual.len())
        .map(|spec| PendingMigration {
            name: spec.migration.name().to_string(),
            impact: spec.impact,
        })
        .collect::<Vec<_>>();
    Ok(MigrationPlan {
        exact: pending.is_empty(),
        pending,
    })
}

async fn lock_migration_inventory<C>(db: &C) -> Result<(), DbErr>
where
    C: ConnectionTrait,
{
    db.execute_unprepared(&format!(
        "SELECT pg_advisory_xact_lock(
            hashtextextended('{MIGRATION_LOCK}:' || current_schema(), 0)
        )"
    ))
    .await?;
    Ok(())
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

    let actual = applied_migration_names(db).await?;
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

async fn applied_migration_names<C>(db: &C) -> Result<Vec<String>, DbErr>
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
        return Ok(Vec::new());
    }

    db.query_all(Statement::from_string(
        DatabaseBackend::Postgres,
        format!("SELECT version FROM {MIGRATION_TABLE} ORDER BY version"),
    ))
    .await?
    .into_iter()
    .map(|row| row.try_get::<String>("", "version"))
    .collect()
}

fn migration_names() -> Vec<String> {
    let mut names = Migrator::migrations()
        .into_iter()
        .map(|migration| migration.name().to_string())
        .collect::<Vec<_>>();
    names.sort();
    names
}
