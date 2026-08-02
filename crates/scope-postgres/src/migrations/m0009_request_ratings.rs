use sea_orm::ConnectionTrait;
use sea_orm_migration::{DbErr, MigrationName, MigrationTrait, SchemaManager};

pub struct Migration;

impl MigrationName for Migration {
    fn name(&self) -> &str {
        "m0009_request_ratings"
    }
}

#[sea_orm_migration::async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .get_connection()
            .execute_unprepared(
                r#"
                    CREATE TABLE scope_request_ratings (
                        id varchar PRIMARY KEY,
                        request_id varchar NOT NULL,
                        rater_user_id varchar NOT NULL,
                        subject_user_id varchar NOT NULL,
                        score integer NOT NULL,
                        reason text NOT NULL,
                        created_at_unix bigint NOT NULL,
                        CONSTRAINT fk_scope_request_ratings_request
                            FOREIGN KEY (request_id) REFERENCES scope_requests(id) ON DELETE CASCADE,
                        CONSTRAINT fk_scope_request_ratings_rater
                            FOREIGN KEY (rater_user_id) REFERENCES scope_users(id),
                        CONSTRAINT fk_scope_request_ratings_subject
                            FOREIGN KEY (subject_user_id) REFERENCES scope_users(id),
                        CONSTRAINT scope_request_rating_participants_distinct
                            CHECK (rater_user_id <> subject_user_id),
                        CONSTRAINT scope_request_rating_score
                            CHECK (score BETWEEN 1 AND 5),
                        CONSTRAINT scope_request_rating_reason
                            CHECK (reason = btrim(reason) AND octet_length(reason) BETWEEN 1 AND 1024),
                        CONSTRAINT scope_request_rating_time
                            CHECK (created_at_unix >= 0),
                        CONSTRAINT scope_request_rating_one_per_rater
                            UNIQUE (request_id, rater_user_id),
                        CONSTRAINT scope_request_rating_one_per_subject
                            UNIQUE (request_id, subject_user_id)
                    );

                    CREATE INDEX idx_scope_request_ratings_subject
                        ON scope_request_ratings (subject_user_id, created_at_unix, id);
                "#,
            )
            .await?;
        Ok(())
    }
}
