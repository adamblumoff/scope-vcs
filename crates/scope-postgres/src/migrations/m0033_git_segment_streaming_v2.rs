use sea_orm::ConnectionTrait;
use sea_orm_migration::{DbErr, MigrationName, MigrationTrait, SchemaManager};

pub struct Migration;

impl MigrationName for Migration {
    fn name(&self) -> &str {
        "m0033_git_segment_streaming_v2"
    }
}

#[sea_orm_migration::async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .get_connection()
            .execute_unprepared(
                "LOCK TABLE scope_git_segments, scope_object_references IN ACCESS EXCLUSIVE MODE;

                DO $$
                BEGIN
                    IF EXISTS (SELECT 1 FROM scope_git_segments) THEN
                        RAISE EXCEPTION USING
                            MESSAGE = 'm0033 requires scope_git_segments to be empty',
                            HINT = 'Reset staging Git repositories before applying the v2 Git segment cutover.';
                    END IF;
                END
                $$;

                DELETE FROM scope_object_references
                WHERE ref_kind = 'git_segment';

                CREATE TABLE scope_git_segment_uploads (
                    segment_id text PRIMARY KEY,
                    repo_id text NOT NULL,
                    object_key text NOT NULL UNIQUE,
                    state text NOT NULL,
                    sha256 text,
                    plaintext_bytes bigint,
                    encrypted_bytes bigint,
                    encoding_version integer NOT NULL,
                    created_at_unix bigint NOT NULL,
                    updated_at_unix bigint NOT NULL,
                    CONSTRAINT scope_git_segment_upload_state CHECK (
                        state IN ('uploading', 'ready', 'published', 'deleting', 'deleted')
                    ),
                    CONSTRAINT scope_git_segment_upload_values CHECK (
                        length(btrim(segment_id)) > 0 AND
                        length(btrim(object_key)) > 0 AND
                        encoding_version > 0 AND
                        created_at_unix >= 0 AND
                        updated_at_unix >= created_at_unix AND
                        (sha256 IS NULL OR length(sha256) = 64) AND
                        (plaintext_bytes IS NULL OR plaintext_bytes >= 0) AND
                        (encrypted_bytes IS NULL OR encrypted_bytes >= 0) AND
                        (
                            state NOT IN ('ready', 'published') OR
                            (sha256 IS NOT NULL AND plaintext_bytes IS NOT NULL AND encrypted_bytes IS NOT NULL)
                        )
                    )
                );

                CREATE INDEX idx_scope_git_segment_uploads_recovery
                    ON scope_git_segment_uploads (state, updated_at_unix, segment_id)
                    WHERE state IN ('uploading', 'ready', 'deleting');

                CREATE TABLE scope_git_segment_references (
                    segment_id text NOT NULL
                        REFERENCES scope_git_segment_uploads(segment_id) ON DELETE RESTRICT,
                    ref_kind text NOT NULL,
                    ref_id text NOT NULL,
                    PRIMARY KEY (segment_id, ref_kind, ref_id),
                    CONSTRAINT scope_git_segment_reference_values CHECK (
                        ref_kind IN ('push_trigger_source', 'run_source') AND
                        length(btrim(ref_id)) > 0
                    )
                );

                CREATE INDEX idx_scope_git_segment_references_owner
                    ON scope_git_segment_references (ref_kind, ref_id, segment_id);

                ALTER TABLE scope_git_segments
                    DROP COLUMN object_key,
                    DROP COLUMN sha256,
                    DROP COLUMN size_bytes,
                    ADD COLUMN segment_id text NOT NULL,
                    ADD CONSTRAINT fk_scope_git_segments_upload
                        FOREIGN KEY (segment_id)
                        REFERENCES scope_git_segment_uploads(segment_id),
                    ADD CONSTRAINT uq_scope_git_segments_segment UNIQUE (segment_id);
                ",
            )
            .await?;
        Ok(())
    }
}
