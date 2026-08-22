use sea_orm::ConnectionTrait;
use sea_orm_migration::{DbErr, MigrationName, MigrationTrait, SchemaManager};

pub struct Migration;

impl MigrationName for Migration {
    fn name(&self) -> &str {
        "m0026_repository_landing_files"
    }
}

#[sea_orm_migration::async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .get_connection()
            .execute_unprepared(
                r#"
                CREATE TABLE scope_repository_landing_files (
                    repo_id text PRIMARY KEY
                        REFERENCES scope_repositories(id) ON DELETE CASCADE,
                    path text NOT NULL,
                    oid text NOT NULL,
                    sha256 text NOT NULL,
                    size_bytes bigint NOT NULL,
                    git_file_mode text NOT NULL,
                    content_bytes bytea NOT NULL,
                    CONSTRAINT scope_repository_landing_file_values CHECK (
                        path = '/README.html' AND
                        char_length(oid) BETWEEN 1 AND 128 AND
                        char_length(sha256) = 64 AND
                        sha256 = lower(sha256) AND
                        sha256 ~ '^[0-9a-f]{64}$' AND
                        size_bytes BETWEEN 0 AND 1048576 AND
                        octet_length(content_bytes) = size_bytes AND
                        git_file_mode IN ('100644', '100755')
                    )
                );
                "#,
            )
            .await?;
        Ok(())
    }
}
