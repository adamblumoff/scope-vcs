use sea_orm::ConnectionTrait;
use sea_orm_migration::{DbErr, MigrationName, MigrationTrait, SchemaManager};

pub struct Migration;

impl MigrationName for Migration {
    fn name(&self) -> &str {
        "m0038_history_entry_positions"
    }
}

#[sea_orm_migration::async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        // Visibility boundaries can split one source update into several history entries.
        // Maintenance excludes writers while we replace the identity and lookup indexes.
        manager
            .get_connection()
            .execute_unprepared(
                r#"
                SET LOCAL lock_timeout = '5s';
                ALTER TABLE scope_repository_history_entries
                    DROP CONSTRAINT scope_repository_history_entries_pkey,
                    DROP CONSTRAINT scope_repository_history_entries_repo_id_audience_position_key,
                    ADD PRIMARY KEY (repo_id, audience, position);
                CREATE INDEX idx_scope_repository_history_entries_source
                    ON scope_repository_history_entries (repo_id, audience, source_id, position DESC);
                "#,
            )
            .await?;
        Ok(())
    }
}
