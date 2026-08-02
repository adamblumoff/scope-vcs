use sea_orm::ConnectionTrait;
use sea_orm_migration::{DbErr, MigrationName, MigrationTrait, SchemaManager};

pub struct Migration;

impl MigrationName for Migration {
    fn name(&self) -> &str {
        "m0007_drop_review_ceremony"
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
                    WHERE kind IN ('Held', 'HoldReleased');

                    DELETE FROM scope_request_events event
                    USING scope_requests request
                    WHERE event.request_id = request.id
                      AND event.kind = 'Assessed'
                      AND request.merged_at_unix IS NOT NULL;

                    UPDATE scope_request_events event
                    SET kind = 'Closed',
                        payload = jsonb_build_object(
                            'Closed',
                            jsonb_build_object(
                                'head_oid', event.payload -> 'Assessed' ->> 'head_oid'
                            )
                        )
                    FROM scope_requests request
                    WHERE event.request_id = request.id
                      AND event.kind = 'Assessed'
                      AND request.merged_at_unix IS NULL;

                    ALTER TABLE scope_requests
                        DROP CONSTRAINT fk_scope_requests_holder,
                        DROP CONSTRAINT fk_scope_requests_assessor,
                        DROP CONSTRAINT scope_request_nonnegative_values,
                        DROP CONSTRAINT scope_request_lifecycle_values,
                        DROP CONSTRAINT scope_request_assessment_values,
                        DROP CONSTRAINT scope_request_assessment_coherence,
                        DROP CONSTRAINT scope_request_merge_coherence,
                        DROP COLUMN held_at_unix,
                        DROP COLUMN held_by_user_id,
                        DROP COLUMN assessment_outcome,
                        DROP COLUMN assessment_body_markdown,
                        DROP COLUMN assessed_at_unix,
                        DROP COLUMN assessed_by_user_id;

                    ALTER TABLE scope_requests
                        ADD CONSTRAINT scope_request_nonnegative_values CHECK (
                            activity_version >= 0 AND
                            created_at_unix >= 0 AND updated_at_unix >= created_at_unix AND
                            (first_ready_at_unix IS NULL OR first_ready_at_unix BETWEEN created_at_unix AND updated_at_unix) AND
                            (ready_at_unix IS NULL OR ready_at_unix BETWEEN created_at_unix AND updated_at_unix) AND
                            (completed_at_unix IS NULL OR completed_at_unix BETWEEN created_at_unix AND updated_at_unix) AND
                            (merged_at_unix IS NULL OR merged_at_unix BETWEEN created_at_unix AND updated_at_unix)
                        ),
                        ADD CONSTRAINT scope_request_lifecycle_values CHECK (
                            state IN ('Working', 'ReadyForReview', 'Completed') AND (
                                (state = 'Working' AND ready_at_unix IS NULL AND
                                 completed_at_unix IS NULL AND completed_by_user_id IS NULL) OR
                                (state = 'ReadyForReview' AND first_ready_at_unix IS NOT NULL AND
                                 ready_at_unix IS NOT NULL AND ready_at_unix >= first_ready_at_unix AND
                                 completed_at_unix IS NULL AND completed_by_user_id IS NULL) OR
                                (state = 'Completed' AND first_ready_at_unix IS NOT NULL AND
                                 ready_at_unix IS NULL AND completed_at_unix IS NOT NULL AND
                                 completed_by_user_id IS NOT NULL AND
                                 completed_at_unix >= first_ready_at_unix)
                            )
                        ),
                        ADD CONSTRAINT scope_request_merge_coherence CHECK (
                            (
                                merged_at_unix IS NULL AND merged_by_user_id IS NULL AND
                                merged_head_oid IS NULL AND merged_main_oid IS NULL
                            ) OR (
                                merged_at_unix IS NOT NULL AND merged_by_user_id IS NOT NULL AND
                                merged_head_oid IS NOT NULL AND length(merged_head_oid) > 0 AND
                                merged_main_oid IS NOT NULL AND length(merged_main_oid) > 0 AND
                                state = 'Completed' AND merged_at_unix >= completed_at_unix
                            )
                        );
                "#,
            )
            .await?;
        Ok(())
    }
}
