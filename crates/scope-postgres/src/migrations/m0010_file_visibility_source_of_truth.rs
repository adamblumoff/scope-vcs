use sea_orm::ConnectionTrait;
use sea_orm_migration::{DbErr, MigrationName, MigrationTrait, SchemaManager};

pub struct Migration;

impl MigrationName for Migration {
    fn name(&self) -> &str {
        "m0010_file_visibility_source_of_truth"
    }
}

#[sea_orm_migration::async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .get_connection()
            .execute_unprepared(
                r#"
                    UPDATE scope_repositories
                    SET publication_state = CASE publication_state
                        WHEN 'Unpublished' THEN 'AwaitingFirstPush'
                        WHEN 'Published' THEN 'Ready'
                        ELSE publication_state
                    END;

                    ALTER TABLE scope_repositories
                        DROP COLUMN default_visibility;
                "#,
            )
            .await?;
        Ok(())
    }
}
