use sea_orm::ConnectionTrait;
use sea_orm_migration::{DbErr, MigrationName, MigrationTrait, SchemaManager};

pub struct Migration;

impl MigrationName for Migration {
    fn name(&self) -> &str {
        "m0022_git_pack_spans"
    }
}

#[sea_orm_migration::async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .get_connection()
            .execute_unprepared(
                r#"
                LOCK TABLE scope_git_heads, scope_git_segments,
                    scope_object_references IN ACCESS EXCLUSIVE MODE;

                DO $$
                BEGIN
                    IF EXISTS (
                        SELECT 1
                        FROM (
                            SELECT repo_id, sequence,
                                row_number() OVER (
                                    PARTITION BY repo_id ORDER BY sequence
                                ) AS expected_sequence
                            FROM scope_git_segments
                        ) ordered
                        WHERE sequence <> expected_sequence
                    ) THEN
                        RAISE EXCEPTION
                            'Git pack span cutover requires contiguous segment sequences starting at 1';
                    END IF;

                    IF EXISTS (
                        SELECT 1
                        FROM scope_git_heads heads
                        LEFT JOIN (
                            SELECT repo_id, max(sequence) AS last_sequence
                            FROM scope_git_segments
                            GROUP BY repo_id
                        ) segments ON segments.repo_id = heads.repo_id
                        WHERE segments.last_sequence IS NULL
                            OR segments.last_sequence <> heads.segment_sequence
                    ) THEN
                        RAISE EXCEPTION
                            'Git pack span cutover requires segment coverage through every Git head';
                    END IF;

                    IF EXISTS (
                        SELECT 1
                        FROM scope_git_segments segments
                        LEFT JOIN scope_git_heads heads
                            ON heads.repo_id = segments.repo_id
                        WHERE heads.repo_id IS NULL
                    ) THEN
                        RAISE EXCEPTION
                            'Git pack span cutover requires a Git head for every segment layout';
                    END IF;
                END $$;

                ALTER TABLE scope_git_heads
                    RENAME COLUMN segment_sequence TO push_sequence;

                INSERT INTO scope_orphan_object_jobs (
                    object_key, generation, sha256, git_oid, size_bytes,
                    attempts, next_run_at_unix, last_error, completed_at_unix,
                    created_at_unix, updated_at_unix
                )
                SELECT DISTINCT ON (segments.manifest_object_key)
                    segments.manifest_object_key,
                    'm0022_git_pack_spans',
                    segments.manifest_sha256,
                    segments.head_oid,
                    segments.manifest_size_bytes,
                    0, 0, NULL, NULL, 0, 0
                FROM scope_git_segments segments
                WHERE NOT EXISTS (
                    SELECT 1
                    FROM scope_object_references refs
                    WHERE refs.object_key = segments.manifest_object_key
                        AND refs.ref_kind <> 'git_segment_manifest'
                )
                ORDER BY segments.manifest_object_key, segments.sequence
                ON CONFLICT (object_key) DO NOTHING;

                DELETE FROM scope_object_references
                WHERE ref_kind = 'git_segment_manifest';

                ALTER TABLE scope_git_segments
                    DROP CONSTRAINT scope_git_segment_values;
                ALTER TABLE scope_git_segments
                    RENAME COLUMN sequence TO first_sequence;

                ALTER TABLE scope_git_segments
                    ADD COLUMN last_sequence bigint,
                    ADD COLUMN geometric_tier integer;

                UPDATE scope_git_segments
                SET last_sequence = first_sequence,
                    geometric_tier = 0;

                ALTER TABLE scope_git_segments
                    ALTER COLUMN last_sequence SET NOT NULL,
                    ALTER COLUMN geometric_tier SET NOT NULL,
                    DROP COLUMN manifest_object_key,
                    DROP COLUMN manifest_sha256,
                    DROP COLUMN manifest_size_bytes,
                    ADD CONSTRAINT scope_git_pack_span_values CHECK (
                        first_sequence > 0 AND
                        last_sequence >= first_sequence AND
                        geometric_tier BETWEEN 0 AND 62 AND
                        size_bytes >= 0 AND
                        last_sequence - first_sequence + 1 =
                            power(2::numeric, geometric_tier)
                    );

                ALTER TABLE scope_git_segments
                    RENAME CONSTRAINT scope_git_segments_pkey
                    TO scope_git_pack_spans_pkey;

                CREATE UNIQUE INDEX scope_git_pack_spans_last_sequence
                    ON scope_git_segments (repo_id, last_sequence);
                "#,
            )
            .await?;
        Ok(())
    }
}
