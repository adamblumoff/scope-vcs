use sea_orm::ConnectionTrait;
use sea_orm_migration::{DbErr, MigrationName, MigrationTrait, SchemaManager};

pub struct Migration;

impl MigrationName for Migration {
    fn name(&self) -> &str {
        "m0024_git_compaction_scheduler"
    }
}

#[sea_orm_migration::async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .get_connection()
            .execute_unprepared(
                r#"
                CREATE TABLE scope_git_compaction_jobs (
                    repo_id text PRIMARY KEY
                        REFERENCES scope_repositories(id) ON DELETE CASCADE,
                    target_sequence bigint NOT NULL,
                    attempts integer NOT NULL DEFAULT 0,
                    next_run_at_unix bigint NOT NULL,
                    lease_generation text,
                    lease_owner text,
                    lease_expires_at_unix bigint,
                    last_error text,
                    created_at_unix bigint NOT NULL,
                    updated_at_unix bigint NOT NULL,
                    CONSTRAINT scope_git_compaction_job_values CHECK (
                        target_sequence > 0 AND
                        attempts >= 0 AND
                        next_run_at_unix >= 0 AND
                        (lease_generation IS NULL) = (lease_owner IS NULL) AND
                        (lease_generation IS NULL) = (lease_expires_at_unix IS NULL) AND
                        (lease_expires_at_unix IS NULL OR lease_expires_at_unix >= 0) AND
                        (last_error IS NULL OR char_length(last_error) BETWEEN 1 AND 2000) AND
                        created_at_unix >= 0 AND
                        updated_at_unix >= created_at_unix
                    )
                );

                CREATE INDEX scope_git_compaction_jobs_due
                    ON scope_git_compaction_jobs (
                        next_run_at_unix, lease_expires_at_unix, repo_id
                    );

                INSERT INTO scope_git_compaction_jobs (
                    repo_id, target_sequence, attempts, next_run_at_unix,
                    lease_generation, lease_owner, lease_expires_at_unix,
                    last_error, created_at_unix, updated_at_unix
                )
                SELECT
                    repo_id,
                    push_sequence,
                    0,
                    extract(epoch FROM transaction_timestamp())::bigint,
                    NULL,
                    NULL,
                    NULL,
                    NULL,
                    extract(epoch FROM transaction_timestamp())::bigint,
                    extract(epoch FROM transaction_timestamp())::bigint
                FROM scope_git_heads;
                "#,
            )
            .await?;
        Ok(())
    }
}
