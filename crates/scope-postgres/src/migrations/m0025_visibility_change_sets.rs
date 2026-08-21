use sea_orm::ConnectionTrait;
use sea_orm_migration::{DbErr, MigrationName, MigrationTrait, SchemaManager};

pub struct Migration;

impl MigrationName for Migration {
    fn name(&self) -> &str {
        "m0025_visibility_change_sets"
    }
}

#[sea_orm_migration::async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .get_connection()
            .execute_unprepared(
                r#"
                CREATE TABLE scope_visibility_change_sets (
                    repo_id character varying NOT NULL
                        REFERENCES scope_repositories(id) ON DELETE CASCADE,
                    id character varying NOT NULL,
                    ordinal bigint NOT NULL,
                    anchor_commit_id character varying,
                    source_update_id character varying,
                    author_id character varying NOT NULL,
                    PRIMARY KEY (repo_id, id),
                    UNIQUE (repo_id, ordinal),
                    CONSTRAINT scope_visibility_change_set_values CHECK (
                        ordinal >= 0 AND char_length(id) > 0 AND char_length(author_id) > 0
                    )
                );

                CREATE TABLE scope_visibility_changes (
                    repo_id character varying NOT NULL,
                    change_set_id character varying NOT NULL,
                    ordinal bigint NOT NULL,
                    path character varying NOT NULL,
                    old_visibility character varying NOT NULL,
                    new_visibility character varying NOT NULL,
                    current_content jsonb,
                    PRIMARY KEY (repo_id, change_set_id, ordinal),
                    UNIQUE (repo_id, change_set_id, path),
                    FOREIGN KEY (repo_id, change_set_id)
                        REFERENCES scope_visibility_change_sets(repo_id, id) ON DELETE CASCADE,
                    CONSTRAINT scope_visibility_change_values CHECK (
                        ordinal >= 0 AND
                        char_length(path) > 0 AND
                        old_visibility IN ('Public', 'Private') AND
                        new_visibility IN ('Public', 'Private') AND
                        old_visibility <> new_visibility
                    )
                );

                CREATE TEMP TABLE visibility_event_migration_mapping ON COMMIT DROP AS
                WITH ordered AS (
                    SELECT
                        event.*,
                        lag(author_id) OVER repo_order AS previous_author_id,
                        lag(source_commit_id) OVER repo_order AS previous_source_commit_id,
                        lag(after_commit_id) OVER repo_order AS previous_after_commit_id,
                        row_number() OVER repo_order AS repo_row
                    FROM scope_visibility_events event
                    WINDOW repo_order AS (PARTITION BY repo_id ORDER BY ordinal)
                ), marked AS (
                    SELECT *,
                        CASE WHEN repo_row = 1
                            OR author_id IS DISTINCT FROM previous_author_id
                            OR source_commit_id IS DISTINCT FROM previous_source_commit_id
                            OR after_commit_id IS DISTINCT FROM previous_after_commit_id
                        THEN 1 ELSE 0 END AS starts_group
                    FROM ordered
                ), grouped AS (
                    SELECT *,
                        sum(starts_group) OVER (
                            PARTITION BY repo_id ORDER BY ordinal ROWS UNBOUNDED PRECEDING
                        ) AS group_number
                    FROM marked
                )
                SELECT
                    repo_id,
                    id AS old_event_id,
                    ordinal AS old_ordinal,
                    'vchg_m' || (min(ordinal) OVER (
                        PARTITION BY repo_id, group_number
                    ))::text AS change_set_id,
                    dense_rank() OVER (
                        PARTITION BY repo_id ORDER BY group_number
                    ) - 1 AS change_set_ordinal,
                    row_number() OVER (
                        PARTITION BY repo_id, group_number ORDER BY ordinal
                    ) - 1 AS child_ordinal,
                    after_commit_id AS anchor_commit_id,
                    source_commit_id AS source_update_id,
                    author_id,
                    path,
                    old_visibility,
                    new_visibility,
                    current_content
                FROM grouped;

                INSERT INTO scope_visibility_change_sets (
                    repo_id, id, ordinal, anchor_commit_id, source_update_id, author_id
                )
                SELECT DISTINCT
                    repo_id, change_set_id, change_set_ordinal,
                    anchor_commit_id, source_update_id, author_id
                FROM visibility_event_migration_mapping;

                INSERT INTO scope_visibility_changes (
                    repo_id, change_set_id, ordinal, path,
                    old_visibility, new_visibility, current_content
                )
                SELECT
                    repo_id, change_set_id, child_ordinal, path,
                    old_visibility, new_visibility, current_content
                FROM visibility_event_migration_mapping
                ORDER BY repo_id, change_set_ordinal, child_ordinal;

                UPDATE scope_object_references reference
                SET ref_kind = 'visibility_change',
                    ref_id = mapping.repo_id || ':' || mapping.change_set_id || ':' || mapping.child_ordinal::text
                FROM visibility_event_migration_mapping mapping
                WHERE reference.ref_kind = 'visibility_event'
                  AND reference.ref_id = mapping.repo_id || ':' || mapping.old_event_id;

                DROP TABLE scope_visibility_events;

                DELETE FROM scope_projection_files;
                DELETE FROM scope_projection_read_models;
                ALTER TABLE scope_projection_read_models
                    DROP CONSTRAINT scope_projection_read_model_identity,
                    ADD CONSTRAINT scope_projection_read_model_identity CHECK (
                        identity_version = 2 AND
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
                    'outbox_projection_identity_v2_' || md5(repo.id || ':' || repo.change_version::text),
                    'projection_read_model_rebuild:' || repo.id || ':' || repo.change_version::text,
                    'projection_read_model_rebuild',
                    repo.id,
                    repo.change_version,
                    jsonb_build_object(
                        'repo_id', repo.id,
                        'repo_version', repo.change_version,
                        'source', 'live'
                    ),
                    'ready', 0, 0, NULL, NULL, NULL, 0, 0, NULL
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
                "#,
            )
            .await?;
        Ok(())
    }
}
