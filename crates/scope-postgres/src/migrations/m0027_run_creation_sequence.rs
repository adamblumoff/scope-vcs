use sea_orm::ConnectionTrait;
use sea_orm_migration::{DbErr, MigrationName, MigrationTrait, SchemaManager};

pub struct Migration;

impl MigrationName for Migration {
    fn name(&self) -> &str {
        "m0027_run_creation_sequence"
    }
}

#[sea_orm_migration::async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .get_connection()
            .execute_unprepared(
                r#"
                CREATE SEQUENCE scope_run_creation_sequence AS bigint;

                ALTER TABLE scope_runs
                    ADD COLUMN creation_sequence bigint;

                WITH ordered_runs AS (
                    SELECT
                        id,
                        row_number() OVER (
                            ORDER BY created_at_unix ASC, id ASC
                        ) AS creation_sequence
                    FROM scope_runs
                )
                UPDATE scope_runs AS run
                SET creation_sequence = ordered.creation_sequence
                FROM ordered_runs AS ordered
                WHERE run.id = ordered.id;

                SELECT setval(
                    'scope_run_creation_sequence',
                    COALESCE((SELECT max(creation_sequence) FROM scope_runs), 0) + 1,
                    FALSE
                );

                ALTER SEQUENCE scope_run_creation_sequence
                    OWNED BY scope_runs.creation_sequence;
                ALTER TABLE scope_runs
                    ALTER COLUMN creation_sequence
                        SET DEFAULT nextval('scope_run_creation_sequence'),
                    ALTER COLUMN creation_sequence SET NOT NULL,
                    ADD CONSTRAINT scope_runs_creation_sequence_unique
                        UNIQUE (creation_sequence);

                DROP INDEX idx_scope_runs_history;
                DROP INDEX idx_scope_runs_workflow_history;
                CREATE INDEX idx_scope_runs_history
                    ON scope_runs (repo_id, creation_sequence DESC);
                CREATE INDEX idx_scope_runs_workflow_history
                    ON scope_runs (
                        repo_id, workflow_path, creation_sequence DESC
                    );
                "#,
            )
            .await?;
        Ok(())
    }
}
