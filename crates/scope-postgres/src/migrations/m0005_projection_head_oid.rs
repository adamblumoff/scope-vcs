use sea_orm::ConnectionTrait;
use sea_orm_migration::{DbErr, MigrationName, MigrationTrait, SchemaManager};

pub struct Migration;

impl MigrationName for Migration {
    fn name(&self) -> &str {
        "m0005_projection_head_oid"
    }
}

#[sea_orm_migration::async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .get_connection()
            .execute_unprepared(
                "
                    ALTER TABLE scope_projection_read_models
                        ADD COLUMN head_oid character varying;

                    DELETE FROM scope_projection_files;
                    DELETE FROM scope_projection_read_models;

                    ALTER TABLE scope_projection_read_models
                        ADD COLUMN identity_version smallint NOT NULL,
                        ADD CONSTRAINT scope_projection_read_model_identity CHECK (
                            identity_version = 1 AND
                            (head_oid IS NULL OR (
                                char_length(head_oid) = 40 AND
                                head_oid ~ '^[0-9A-Fa-f]+$'
                            ))
                        );

                    INSERT INTO scope_outbox_jobs (
                        id, idempotency_key, kind, repo_id, repo_version, payload,
                        state, attempts, next_run_at_unix, lease_owner,
                        lease_expires_at_unix, last_error, created_at_unix,
                        updated_at_unix, completed_at_unix
                    )
                    SELECT
                        'outbox_projection_identity_' || md5(repo.id || ':' || repo.change_version::text),
                        'projection_read_model_rebuild:' || repo.id || ':' || repo.change_version::text,
                        'projection_read_model_rebuild',
                        repo.id,
                        repo.change_version,
                        jsonb_build_object(
                            'repo_id', repo.id,
                            'repo_version', repo.change_version,
                            'source', 'live'
                        ),
                        'ready',
                        0,
                        0,
                        NULL,
                        NULL,
                        NULL,
                        0,
                        0,
                        NULL
                    FROM scope_repositories repo
                    ON CONFLICT (idempotency_key) DO UPDATE
                    SET kind = EXCLUDED.kind,
                        repo_id = EXCLUDED.repo_id,
                        repo_version = EXCLUDED.repo_version,
                        payload = EXCLUDED.payload,
                        state = EXCLUDED.state,
                        attempts = EXCLUDED.attempts,
                        next_run_at_unix = EXCLUDED.next_run_at_unix,
                        lease_owner = NULL,
                        lease_expires_at_unix = NULL,
                        last_error = NULL,
                        updated_at_unix = EXCLUDED.updated_at_unix,
                        completed_at_unix = NULL;
                ",
            )
            .await?;
        Ok(())
    }
}
