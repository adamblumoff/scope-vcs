use sea_orm::ConnectionTrait;
use sea_orm_migration::{DbErr, MigrationName, MigrationTrait, SchemaManager};

pub struct Migration;

impl MigrationName for Migration {
    fn name(&self) -> &str {
        "m0035_request_queue_indexes"
    }
}

#[sea_orm_migration::async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .get_connection()
            .execute_unprepared(
                r#"
                -- Test schemas share a database-wide extension. Serialize its
                -- first installation even when their migrations run together.
                SELECT pg_advisory_xact_lock(hashtextextended('scope:pg-trgm-install', 0));
                CREATE EXTENSION IF NOT EXISTS pg_trgm WITH SCHEMA public;

                CREATE INDEX idx_scope_requests_open_queue
                    ON scope_requests (repo_id, submitted_at_unix, id)
                    WHERE submitted_at_unix IS NOT NULL
                        AND closed_at_unix IS NULL AND merged_at_unix IS NULL;
                CREATE INDEX idx_scope_requests_public_open_queue
                    ON scope_requests (repo_id, submitted_at_unix, id)
                    WHERE audience = 'Public' AND submitted_at_unix IS NOT NULL
                        AND closed_at_unix IS NULL AND merged_at_unix IS NULL;
                CREATE INDEX idx_scope_requests_draft_queue
                    ON scope_requests (repo_id, updated_at_unix DESC, id)
                    WHERE submitted_at_unix IS NULL
                        AND closed_at_unix IS NULL AND merged_at_unix IS NULL;
                CREATE INDEX idx_scope_requests_closed_queue
                    ON scope_requests (repo_id, (COALESCE(closed_at_unix, merged_at_unix)) DESC, id)
                    WHERE closed_at_unix IS NOT NULL OR merged_at_unix IS NOT NULL;
                CREATE INDEX idx_scope_requests_public_closed_queue
                    ON scope_requests (repo_id, (COALESCE(closed_at_unix, merged_at_unix)) DESC, id)
                    WHERE audience = 'Public'
                        AND (closed_at_unix IS NOT NULL OR merged_at_unix IS NOT NULL);
                CREATE INDEX idx_scope_requests_public_search
                    ON scope_requests USING gin (
                        title public.gin_trgm_ops,
                        description_markdown public.gin_trgm_ops
                    )
                    WHERE audience = 'Public';
                "#,
            )
            .await?;
        Ok(())
    }
}
