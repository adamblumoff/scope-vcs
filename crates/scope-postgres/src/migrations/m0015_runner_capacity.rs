use sea_orm::ConnectionTrait;
use sea_orm_migration::{DbErr, MigrationName, MigrationTrait, SchemaManager};

pub struct Migration;

impl MigrationName for Migration {
    fn name(&self) -> &str {
        "m0015_runner_capacity"
    }
}

#[sea_orm_migration::async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .get_connection()
            .execute_unprepared(
                "ALTER TABLE scope_runners ADD COLUMN max_concurrent_jobs integer;
                 UPDATE scope_runners SET max_concurrent_jobs = 1;
                 ALTER TABLE scope_runners
                     ALTER COLUMN max_concurrent_jobs SET NOT NULL,
                     ADD CONSTRAINT scope_runners_capacity CHECK (
                         max_concurrent_jobs BETWEEN 1 AND 16
                     );",
            )
            .await?;
        Ok(())
    }
}
