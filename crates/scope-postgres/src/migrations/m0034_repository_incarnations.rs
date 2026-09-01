use sea_orm::ConnectionTrait;
use sea_orm_migration::{DbErr, MigrationName, MigrationTrait, SchemaManager};

pub struct Migration;

impl MigrationName for Migration {
    fn name(&self) -> &str {
        "m0034_repository_incarnations"
    }
}

#[sea_orm_migration::async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .get_connection()
            .execute_unprepared(
                r#"
                LOCK TABLE scope_repositories, scope_repo_storage_cleanup_jobs
                    IN ACCESS EXCLUSIVE MODE;

                ALTER TABLE scope_repositories
                    ADD COLUMN incarnation_id text;
                UPDATE scope_repositories
                SET incarnation_id = 'repoi_m0034_' || md5(id);
                ALTER TABLE scope_repositories
                    ALTER COLUMN incarnation_id SET NOT NULL,
                    ADD CONSTRAINT uq_scope_repositories_incarnation
                        UNIQUE (incarnation_id),
                    ADD CONSTRAINT scope_repository_incarnation_nonempty
                        CHECK (length(btrim(incarnation_id)) > 0);

                ALTER TABLE scope_repo_storage_cleanup_jobs
                    ADD COLUMN incarnation_id text;
                UPDATE scope_repo_storage_cleanup_jobs
                SET incarnation_id = 'repoi_m0034_cleanup_' || md5(repo_id || ':' || generation);
                ALTER TABLE scope_repo_storage_cleanup_jobs
                    ALTER COLUMN incarnation_id SET NOT NULL,
                    ADD CONSTRAINT uq_scope_repo_cleanup_incarnation
                        UNIQUE (incarnation_id),
                    ADD CONSTRAINT scope_repo_cleanup_incarnation_nonempty
                        CHECK (length(btrim(incarnation_id)) > 0);
                "#,
            )
            .await?;
        Ok(())
    }
}
