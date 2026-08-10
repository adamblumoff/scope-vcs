use sea_orm::ConnectionTrait;
use sea_orm_migration::{DbErr, MigrationName, MigrationTrait, SchemaManager};

pub struct Migration;

impl MigrationName for Migration {
    fn name(&self) -> &str {
        "m0019_run_attempt_cache_observations"
    }
}

#[sea_orm_migration::async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .get_connection()
            .execute_unprepared(
                "CREATE TABLE scope_run_attempt_caches (
                    attempt_id text NOT NULL REFERENCES scope_run_attempts(id) ON DELETE CASCADE,
                    identity_digest varchar(64) NOT NULL,
                    workflow_path text NOT NULL,
                    job_key varchar(64) NOT NULL,
                    cache_name varchar(64) NOT NULL,
                    preparation text NOT NULL,
                    cold_reason text,
                    prepare_ms bigint NOT NULL,
                    final_state text NOT NULL,
                    finalize_ms bigint,
                    PRIMARY KEY (attempt_id, identity_digest),
                    CONSTRAINT scope_run_attempt_caches_identity_digest CHECK (
                        identity_digest ~ '^[0-9a-f]{64}$'
                    ),
                    CONSTRAINT scope_run_attempt_caches_preparation CHECK (
                        (preparation = 'warm' AND cold_reason IS NULL)
                        OR (
                            preparation = 'cold'
                            AND cold_reason IN (
                                'metadata-missing',
                                'metadata-invalid',
                                'metadata-not-ready',
                                'volume-missing',
                                'volume-invalid',
                                'backing-directory-missing'
                            )
                        )
                    ),
                    CONSTRAINT scope_run_attempt_caches_prepare_duration CHECK (
                        prepare_ms BETWEEN 0 AND 86400000
                    ),
                    CONSTRAINT scope_run_attempt_caches_finalization CHECK (
                        (final_state = 'pending' AND finalize_ms IS NULL)
                        OR (
                            final_state IN ('ready', 'evicted')
                            AND finalize_ms BETWEEN 0 AND 86400000
                        )
                    )
                )",
            )
            .await?;
        Ok(())
    }
}
