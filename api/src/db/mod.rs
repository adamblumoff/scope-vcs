#![allow(dead_code)]

pub(crate) mod entities;
mod migrations;

use anyhow::Context;
use sea_orm::{ConnectOptions, Database, DatabaseConnection};
use sea_orm_migration::MigratorTrait;
use std::time::Duration;

pub(crate) use migrations::Migrator;

pub(crate) async fn connect(database_url: &str) -> anyhow::Result<DatabaseConnection> {
    let mut options = ConnectOptions::new(database_url.to_string());
    options
        .max_connections(20)
        .min_connections(1)
        .connect_timeout(Duration::from_secs(8))
        .acquire_timeout(Duration::from_secs(8))
        .idle_timeout(Duration::from_secs(300))
        .sqlx_logging(false);

    Database::connect(options)
        .await
        .context("connecting to Postgres")
}

pub(crate) async fn migrate(db: &DatabaseConnection) -> anyhow::Result<()> {
    Migrator::up(db, None)
        .await
        .context("running database migrations")
}

#[cfg(test)]
pub(crate) fn mock_connection() -> DatabaseConnection {
    sea_orm::MockDatabase::new(sea_orm::DatabaseBackend::Postgres).into_connection()
}
