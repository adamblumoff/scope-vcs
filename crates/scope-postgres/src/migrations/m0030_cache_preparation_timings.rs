use sea_orm::ConnectionTrait;
use sea_orm_migration::{DbErr, MigrationName, MigrationTrait, SchemaManager};

pub struct Migration;

impl MigrationName for Migration {
    fn name(&self) -> &str {
        "m0030_cache_preparation_timings"
    }
}

#[sea_orm_migration::async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .get_connection()
            .execute_unprepared(
                "LOCK TABLE scope_run_attempts, scope_run_attempt_caches IN ACCESS EXCLUSIVE MODE;

                TRUNCATE TABLE scope_run_attempt_caches;

                ALTER TABLE scope_run_attempt_caches
                    ADD COLUMN key_ms bigint NOT NULL,
                    ADD COLUMN metadata_ms bigint NOT NULL,
                    ADD COLUMN size_bytes bigint NOT NULL,
                    ADD COLUMN download_verify_ms bigint NOT NULL,
                    ADD COLUMN sync_ms bigint NOT NULL,
                    ADD COLUMN extraction_ms bigint NOT NULL,
                    ADD CONSTRAINT scope_run_attempt_caches_preparation_timings CHECK (
                        key_ms BETWEEN 0 AND 86400000 AND
                        metadata_ms BETWEEN 0 AND 86400000 AND
                        size_bytes BETWEEN 0 AND 1073741824 AND
                        download_verify_ms BETWEEN 0 AND 86400000 AND
                        sync_ms BETWEEN 0 AND 86400000 AND
                        extraction_ms BETWEEN 0 AND 86400000 AND
                        prepare_ms = key_ms + metadata_ms + download_verify_ms + sync_ms + extraction_ms
                    );

                CREATE TABLE scope_run_attempt_cache_setups (
                    attempt_id text PRIMARY KEY REFERENCES scope_run_attempts(id) ON DELETE CASCADE,
                    authorization_ms bigint NOT NULL,
                    wall_ms bigint NOT NULL,
                    CONSTRAINT scope_run_attempt_cache_setups_timings CHECK (
                        authorization_ms BETWEEN 0 AND 86400000 AND
                        wall_ms BETWEEN 0 AND 86400000 AND
                        authorization_ms <= wall_ms
                    )
                )",
            )
            .await?;
        Ok(())
    }
}
