use sea_orm::ConnectionTrait;
use sea_orm_migration::{DbErr, MigrationName, MigrationTrait, SchemaManager};

pub struct Migration;

impl MigrationName for Migration {
    fn name(&self) -> &str {
        "m0012_request_revisions"
    }
}

#[sea_orm_migration::async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .get_connection()
            .execute_unprepared(
                r#"
                    LOCK TABLE scope_request_change_blocks,
                               scope_request_discussions,
                               scope_request_discussion_replies,
                               scope_request_discussion_read_states,
                               scope_object_references
                    IN ACCESS EXCLUSIVE MODE;

                    ALTER TABLE scope_request_change_blocks
                        RENAME TO scope_request_revisions;
                    ALTER TABLE scope_request_revisions
                        RENAME CONSTRAINT scope_request_change_blocks_pkey
                        TO scope_request_revisions_pkey;
                    ALTER TABLE scope_request_revisions
                        RENAME CONSTRAINT scope_request_change_blocks_position_key
                        TO scope_request_revisions_position_key;
                    ALTER TABLE scope_request_revisions
                        RENAME CONSTRAINT fk_scope_request_change_blocks_request
                        TO fk_scope_request_revisions_request;
                    ALTER TABLE scope_request_revisions
                        RENAME CONSTRAINT fk_scope_request_change_blocks_actor
                        TO fk_scope_request_revisions_actor;
                    ALTER TABLE scope_request_revisions
                        RENAME CONSTRAINT scope_request_change_block_values
                        TO scope_request_revision_values;
                    ALTER INDEX idx_scope_request_change_blocks_request_position
                        RENAME TO idx_scope_request_revisions_request_position;

                    UPDATE scope_object_references
                    SET ref_kind = 'request_revision_snapshot'
                    WHERE ref_kind = 'request_change_block_snapshot';

                    ALTER TABLE scope_request_discussions
                        ADD COLUMN revision_id character varying,
                        ADD COLUMN commit_oid character varying,
                        ADD COLUMN path text;
                    ALTER TABLE scope_request_discussions
                        DROP CONSTRAINT scope_request_discussion_values;

                    CREATE TEMPORARY TABLE scope_promoted_request_discussions
                    ON COMMIT DROP AS
                    SELECT discussion.id AS discussion_id,
                           discussion.subject #>> '{ChangeBlock,change_block_id}' AS revision_id,
                           first_reply.id AS reply_id,
                           first_reply.position,
                           first_reply.author_user_id,
                           first_reply.body_markdown,
                           first_reply.created_at_unix
                    FROM scope_request_discussions discussion
                    JOIN LATERAL (
                        SELECT reply.*
                        FROM scope_request_discussion_replies reply
                        WHERE reply.discussion_id = discussion.id
                        ORDER BY reply.position, reply.id
                        LIMIT 1
                    ) first_reply ON TRUE
                    WHERE discussion.subject ? 'ChangeBlock';

                    DELETE FROM scope_request_discussions discussion
                    WHERE discussion.subject ? 'ChangeBlock'
                      AND NOT EXISTS (
                          SELECT 1
                          FROM scope_request_discussion_replies reply
                          WHERE reply.discussion_id = discussion.id
                      );

                    WITH RECURSIVE descendants AS (
                        SELECT reply.id
                        FROM scope_request_discussion_replies reply
                        JOIN scope_promoted_request_discussions promoted
                          ON reply.reply_to_reply_id = promoted.reply_id
                        UNION ALL
                        SELECT child.id
                        FROM scope_request_discussion_replies child
                        JOIN descendants parent ON child.reply_to_reply_id = parent.id
                    )
                    UPDATE scope_request_discussion_replies reply
                    SET depth = depth - 1
                    WHERE reply.id IN (SELECT id FROM descendants);

                    UPDATE scope_request_discussion_replies reply
                    SET reply_to_reply_id = NULL
                    FROM scope_promoted_request_discussions promoted
                    WHERE reply.reply_to_reply_id = promoted.reply_id;

                    UPDATE scope_request_discussions discussion
                    SET opened_position = promoted.position,
                        author_user_id = promoted.author_user_id,
                        body_markdown = promoted.body_markdown,
                        created_at_unix = promoted.created_at_unix,
                        revision_id = promoted.revision_id
                    FROM scope_promoted_request_discussions promoted
                    WHERE discussion.id = promoted.discussion_id;

                    DELETE FROM scope_request_discussion_replies reply
                    USING scope_promoted_request_discussions promoted
                    WHERE reply.id = promoted.reply_id;

                    ALTER TABLE scope_request_discussions
                        DROP COLUMN subject;
                    ALTER TABLE scope_request_discussions
                        ALTER COLUMN body_markdown SET NOT NULL;
                    ALTER TABLE scope_request_revisions
                        ADD CONSTRAINT scope_request_revisions_request_id_key
                        UNIQUE (request_id, id);
                    ALTER TABLE scope_request_discussions
                        ADD CONSTRAINT fk_scope_request_discussions_revision
                        FOREIGN KEY (request_id, revision_id)
                        REFERENCES scope_request_revisions(request_id, id)
                        ON DELETE CASCADE;
                    ALTER TABLE scope_request_discussions
                        ADD CONSTRAINT scope_request_discussion_values CHECK (
                            opened_position > 0 AND
                            last_activity_position >= opened_position AND
                            status IN ('Open', 'Resolved') AND
                            length(btrim(body_markdown)) > 0 AND
                            created_at_unix >= 0 AND
                            (resolved_at_unix IS NULL OR resolved_at_unix >= 0) AND
                            (commit_oid IS NULL OR revision_id IS NOT NULL) AND
                            (path IS NULL OR commit_oid IS NOT NULL)
                        );
                "#,
            )
            .await?;
        Ok(())
    }
}
