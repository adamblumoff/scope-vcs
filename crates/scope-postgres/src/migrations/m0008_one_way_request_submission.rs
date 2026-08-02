use sea_orm::ConnectionTrait;
use sea_orm_migration::{DbErr, MigrationName, MigrationTrait, SchemaManager};

pub struct Migration;

impl MigrationName for Migration {
    fn name(&self) -> &str {
        "m0008_one_way_request_submission"
    }
}

#[sea_orm_migration::async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .get_connection()
            .execute_unprepared(
                r#"
                    WITH ranked_ready AS (
                        SELECT id,
                               row_number() OVER (
                                   PARTITION BY request_id
                                   ORDER BY position ASC, id ASC
                               ) AS publication_number
                        FROM scope_request_events
                        WHERE kind = 'ReadyForReview'
                    )
                    DELETE FROM scope_request_events event
                    USING ranked_ready ranked
                    WHERE event.id = ranked.id
                      AND ranked.publication_number > 1;

                    UPDATE scope_request_events
                    SET kind = 'Submitted',
                        payload = jsonb_build_object(
                            'Submitted', payload -> 'ReadyForReview'
                        )
                    WHERE kind = 'ReadyForReview';

                    DELETE FROM scope_request_events
                    WHERE kind = 'ReturnedToWorking';

                    ALTER TABLE scope_requests
                        DROP CONSTRAINT scope_request_nonnegative_values,
                        DROP CONSTRAINT scope_request_lifecycle_values,
                        DROP CONSTRAINT scope_request_completion_coherence,
                        DROP CONSTRAINT scope_request_merge_coherence;

                    ALTER TABLE scope_requests
                        RENAME COLUMN first_ready_at_unix TO submitted_at_unix;

                    ALTER TABLE scope_requests
                        RENAME COLUMN completed_at_unix TO closed_at_unix;

                    ALTER TABLE scope_requests
                        RENAME COLUMN completed_by_user_id TO closed_by_user_id;

                    UPDATE scope_requests
                    SET submitted_at_unix = CASE
                            WHEN state = 'Working' AND submitted_at_unix IS NULL THEN NULL
                            ELSE submitted_at_unix
                        END,
                        closed_at_unix = CASE
                            WHEN state = 'Completed' AND merged_at_unix IS NULL THEN closed_at_unix
                            ELSE NULL
                        END,
                        closed_by_user_id = CASE
                            WHEN state = 'Completed' AND merged_at_unix IS NULL THEN closed_by_user_id
                            ELSE NULL
                        END;

                    ALTER TABLE scope_requests
                        DROP COLUMN state,
                        DROP COLUMN ready_at_unix;

                    ALTER TABLE scope_requests
                        RENAME CONSTRAINT fk_scope_requests_completer TO fk_scope_requests_closer;

                    ALTER TABLE scope_requests
                        ADD CONSTRAINT scope_request_nonnegative_values CHECK (
                            activity_version >= 0 AND
                            created_at_unix >= 0 AND updated_at_unix >= created_at_unix AND
                            (submitted_at_unix IS NULL OR submitted_at_unix BETWEEN created_at_unix AND updated_at_unix) AND
                            (closed_at_unix IS NULL OR closed_at_unix BETWEEN created_at_unix AND updated_at_unix) AND
                            (merged_at_unix IS NULL OR merged_at_unix BETWEEN created_at_unix AND updated_at_unix)
                        ),
                        ADD CONSTRAINT scope_request_submission_coherence CHECK (
                            (closed_at_unix IS NULL OR merged_at_unix IS NULL) AND
                            (closed_at_unix IS NULL) = (closed_by_user_id IS NULL) AND
                            (
                                (submitted_at_unix IS NULL AND closed_at_unix IS NULL AND merged_at_unix IS NULL) OR
                                (submitted_at_unix IS NOT NULL AND
                                 (closed_at_unix IS NULL OR closed_at_unix >= submitted_at_unix) AND
                                 (merged_at_unix IS NULL OR merged_at_unix >= submitted_at_unix))
                            )
                        ),
                        ADD CONSTRAINT scope_request_merge_coherence CHECK (
                            (
                                merged_at_unix IS NULL AND merged_by_user_id IS NULL AND
                                merged_head_oid IS NULL AND merged_main_oid IS NULL
                            ) OR (
                                submitted_at_unix IS NOT NULL AND
                                merged_at_unix IS NOT NULL AND merged_by_user_id IS NOT NULL AND
                                merged_head_oid IS NOT NULL AND length(merged_head_oid) > 0 AND
                                merged_main_oid IS NOT NULL AND length(merged_main_oid) > 0
                            )
                        );
                "#,
            )
            .await?;
        Ok(())
    }
}
