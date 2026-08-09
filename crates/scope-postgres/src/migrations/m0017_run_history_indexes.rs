use sea_orm::ConnectionTrait;
use sea_orm_migration::{DbErr, MigrationName, MigrationTrait, SchemaManager};

pub struct Migration;

impl MigrationName for Migration {
    fn name(&self) -> &str {
        "m0017_run_history_indexes"
    }
}

#[sea_orm_migration::async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .get_connection()
            .execute_unprepared(
                "DROP INDEX idx_scope_runs_repo;
                 CREATE INDEX idx_scope_runs_history
                     ON scope_runs (repo_id, created_at_unix DESC, id DESC);
                 CREATE INDEX idx_scope_runs_workflow_history
                     ON scope_runs (
                         repo_id, workflow_path, created_at_unix DESC, id DESC
                     );",
            )
            .await?;
        Ok(())
    }
}
