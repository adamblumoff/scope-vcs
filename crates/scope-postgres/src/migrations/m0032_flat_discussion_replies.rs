use sea_orm::ConnectionTrait;
use sea_orm_migration::{DbErr, MigrationName, MigrationTrait, SchemaManager};

pub struct Migration;

impl MigrationName for Migration {
    fn name(&self) -> &str {
        "m0032_flat_discussion_replies"
    }
}

#[sea_orm_migration::async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .get_connection()
            .execute_unprepared(
                "LOCK TABLE scope_request_discussion_replies IN ACCESS EXCLUSIVE MODE;

                DROP INDEX idx_scope_request_discussion_replies_position;
                DROP INDEX idx_scope_request_discussion_replies_tree;
                DROP INDEX idx_scope_request_discussion_replies_parent;

                ALTER TABLE scope_request_discussion_replies
                    DROP CONSTRAINT scope_request_discussion_reply_values,
                    DROP COLUMN depth,
                    ADD CONSTRAINT scope_request_discussion_reply_values CHECK (
                        position > 0 AND
                        length(btrim(body_markdown)) > 0 AND
                        created_at_unix >= 0
                    );

                CREATE INDEX idx_scope_request_discussion_replies_chronological
                    ON scope_request_discussion_replies (discussion_id, position DESC);",
            )
            .await?;
        Ok(())
    }
}
