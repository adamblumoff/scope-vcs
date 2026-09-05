use sea_orm::ConnectionTrait;
use sea_orm_migration::{DbErr, MigrationName, MigrationTrait, SchemaManager};

pub struct Migration;
impl MigrationName for Migration {
    fn name(&self) -> &str {
        "m0037_repository_history_views"
    }
}

#[sea_orm_migration::async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager.get_connection().execute_unprepared(r#"
            CREATE TABLE scope_repository_history_views (
                repo_id text NOT NULL REFERENCES scope_repositories(id) ON DELETE CASCADE,
                audience text NOT NULL CHECK (audience IN ('private', 'public')),
                repo_version bigint NOT NULL CHECK (repo_version >= 0),
                generation text NOT NULL,
                identity_version smallint NOT NULL,
                available boolean NOT NULL,
                visible_files boolean NOT NULL,
                head_oid text,
                PRIMARY KEY (repo_id, audience)
            );
            CREATE TABLE scope_repository_history_entries (
                repo_id text NOT NULL,
                audience text NOT NULL,
                position bigint NOT NULL CHECK (position >= 0),
                source_id text NOT NULL,
                payload jsonb NOT NULL,
                PRIMARY KEY (repo_id, audience, source_id),
                UNIQUE (repo_id, audience, position),
                FOREIGN KEY (repo_id, audience) REFERENCES scope_repository_history_views(repo_id, audience) ON DELETE CASCADE
            );
        "#).await?;
        Ok(())
    }
}
