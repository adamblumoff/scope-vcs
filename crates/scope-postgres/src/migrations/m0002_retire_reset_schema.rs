use sea_orm::ConnectionTrait;
use sea_orm_migration::{DbErr, MigrationName, MigrationTrait, SchemaManager};

pub struct Migration;

impl MigrationName for Migration {
    fn name(&self) -> &str {
        "m0002_retire_reset_schema"
    }
}

#[sea_orm_migration::async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        // This is the deliberate forward-only cutover from destructive v6
        // startup resets. Pre-migration binaries must not be restarted after
        // this commits; the rollout uses a maintenance window and moves
        // forward with corrective migrations.
        manager
            .get_connection()
            .execute_unprepared(
                "
                    DROP TABLE scope_metadata_reset_events;
                    DROP TABLE scope_metadata_schema;
                ",
            )
            .await?;
        Ok(())
    }
}
