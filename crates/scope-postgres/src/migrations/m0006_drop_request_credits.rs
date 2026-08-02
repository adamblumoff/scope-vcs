use sea_orm::ConnectionTrait;
use sea_orm_migration::{DbErr, MigrationName, MigrationTrait, SchemaManager};

pub struct Migration;

impl MigrationName for Migration {
    fn name(&self) -> &str {
        "m0006_drop_request_credits"
    }
}

#[sea_orm_migration::async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .get_connection()
            .execute_unprepared(
                r#"
                    DELETE FROM scope_request_events
                    WHERE kind = 'Settled';

                    UPDATE scope_request_events
                    SET payload = payload #- '{ReadyForReview,stake_credits}'
                    WHERE kind = 'ReadyForReview';

                    UPDATE scope_request_events
                    SET payload = payload #- '{ReturnedToWorking,stake_credits}'
                    WHERE kind = 'ReturnedToWorking';

                    UPDATE scope_request_events
                    SET payload = payload #- '{Assessed,stake_credits}'
                    WHERE kind = 'Assessed';

                    DROP TABLE scope_credit_ledger_entries;
                    DROP TABLE scope_user_credit_accounts;

                    DROP INDEX idx_scope_requests_ready_queue;
                    ALTER TABLE scope_requests
                        DROP CONSTRAINT scope_request_nonnegative_values,
                        DROP CONSTRAINT scope_request_lifecycle_values,
                        DROP COLUMN current_stake_credits,
                        DROP COLUMN ready_queue_version;

                    ALTER TABLE scope_requests
                        ADD CONSTRAINT scope_request_nonnegative_values CHECK (
                            activity_version >= 0 AND
                            created_at_unix >= 0 AND updated_at_unix >= created_at_unix AND
                            (first_ready_at_unix IS NULL OR first_ready_at_unix BETWEEN created_at_unix AND updated_at_unix) AND
                            (ready_at_unix IS NULL OR ready_at_unix BETWEEN created_at_unix AND updated_at_unix) AND
                            (held_at_unix IS NULL OR held_at_unix BETWEEN created_at_unix AND updated_at_unix) AND
                            (assessed_at_unix IS NULL OR assessed_at_unix BETWEEN created_at_unix AND updated_at_unix) AND
                            (completed_at_unix IS NULL OR completed_at_unix BETWEEN created_at_unix AND updated_at_unix) AND
                            (merged_at_unix IS NULL OR merged_at_unix BETWEEN created_at_unix AND updated_at_unix)
                        ),
                        ADD CONSTRAINT scope_request_lifecycle_values CHECK (
                            state IN ('Working', 'ReadyForReview', 'Completed') AND
                            (
                                (state = 'Working' AND ready_at_unix IS NULL AND
                                 held_at_unix IS NULL AND held_by_user_id IS NULL) OR
                                (state = 'ReadyForReview' AND first_ready_at_unix IS NOT NULL AND
                                 ready_at_unix IS NOT NULL AND ready_at_unix >= first_ready_at_unix) OR
                                (state = 'Completed' AND first_ready_at_unix IS NOT NULL AND
                                 ready_at_unix IS NULL AND held_at_unix IS NULL AND
                                 held_by_user_id IS NULL AND completed_at_unix >= first_ready_at_unix)
                            ) AND
                            (
                                (held_at_unix IS NULL AND held_by_user_id IS NULL) OR
                                (held_at_unix IS NOT NULL AND held_by_user_id IS NOT NULL AND
                                 state = 'ReadyForReview' AND held_at_unix >= ready_at_unix)
                            )
                        );

                    CREATE INDEX idx_scope_requests_ready_queue
                    ON scope_requests (repo_id, first_ready_at_unix, id)
                    WHERE state = 'ReadyForReview';
                "#,
            )
            .await?;
        Ok(())
    }
}
