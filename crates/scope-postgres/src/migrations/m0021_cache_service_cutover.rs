use sea_orm::ConnectionTrait;
use sea_orm_migration::{DbErr, MigrationName, MigrationTrait, SchemaManager};

pub struct Migration;

impl MigrationName for Migration {
    fn name(&self) -> &str {
        "m0021_cache_service_cutover"
    }
}

#[sea_orm_migration::async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .get_connection()
            .execute_unprepared(
                r#"
                CREATE TABLE scope_cache_objects (
                    repository_id text NOT NULL
                        REFERENCES scope_repositories(id) ON DELETE CASCADE,
                    checksum_sha256 varchar(64) NOT NULL,
                    storage_backend varchar(64) NOT NULL,
                    object_key text NOT NULL UNIQUE,
                    size_bytes bigint NOT NULL,
                    created_at_unix bigint NOT NULL,
                    last_accessed_at_unix bigint NOT NULL,
                    PRIMARY KEY (repository_id, checksum_sha256),
                    CONSTRAINT scope_cache_objects_values CHECK (
                        checksum_sha256 ~ '^[0-9a-f]{64}$' AND
                        char_length(storage_backend) BETWEEN 1 AND 64 AND
                        storage_backend ~ '^[a-z0-9]+(-[a-z0-9]+)*$' AND
                        object_key = 'repos/' || repository_id ||
                            '/objects/sha256/' || checksum_sha256 AND
                        size_bytes BETWEEN 1 AND 1073741824 AND
                        created_at_unix >= 0 AND
                        last_accessed_at_unix >= created_at_unix
                    )
                );

                CREATE INDEX idx_scope_cache_objects_access
                    ON scope_cache_objects (
                        repository_id, last_accessed_at_unix, checksum_sha256
                    );

                CREATE TABLE scope_cache_references (
                    repository_id text NOT NULL,
                    identity_digest varchar(64) NOT NULL,
                    checksum_sha256 varchar(64) NOT NULL,
                    version bigint NOT NULL,
                    expires_at_unix bigint NOT NULL,
                    last_accessed_at_unix bigint NOT NULL,
                    PRIMARY KEY (repository_id, identity_digest),
                    CONSTRAINT fk_scope_cache_references_object
                        FOREIGN KEY (repository_id, checksum_sha256)
                        REFERENCES scope_cache_objects (
                            repository_id, checksum_sha256
                        ) ON DELETE CASCADE,
                    CONSTRAINT scope_cache_references_values CHECK (
                        identity_digest ~ '^[0-9a-f]{64}$' AND
                        checksum_sha256 ~ '^[0-9a-f]{64}$' AND
                        version > 0 AND
                        last_accessed_at_unix >= 0 AND
                        expires_at_unix > last_accessed_at_unix
                    )
                );

                CREATE INDEX idx_scope_cache_references_object
                    ON scope_cache_references (repository_id, checksum_sha256);
                CREATE INDEX idx_scope_cache_references_expiry
                    ON scope_cache_references (
                        expires_at_unix, repository_id, identity_digest
                    );
                CREATE INDEX idx_scope_cache_references_access
                    ON scope_cache_references (
                        repository_id, last_accessed_at_unix, identity_digest
                    );

                CREATE TABLE scope_cache_uploads (
                    upload_id text PRIMARY KEY,
                    repository_id text NOT NULL
                        REFERENCES scope_repositories(id) ON DELETE CASCADE,
                    identity_digest varchar(64) NOT NULL,
                    checksum_sha256 varchar(64) NOT NULL,
                    storage_backend varchar(64) NOT NULL,
                    object_key text NOT NULL UNIQUE,
                    size_bytes bigint NOT NULL,
                    expected_reference_version bigint,
                    state text NOT NULL,
                    created_at_unix bigint NOT NULL,
                    expires_at_unix bigint NOT NULL,
                    CONSTRAINT scope_cache_uploads_values CHECK (
                        char_length(upload_id) BETWEEN 1 AND 128 AND
                        upload_id !~ '[[:space:]]' AND
                        identity_digest ~ '^[0-9a-f]{64}$' AND
                        checksum_sha256 ~ '^[0-9a-f]{64}$' AND
                        char_length(storage_backend) BETWEEN 1 AND 64 AND
                        storage_backend ~ '^[a-z0-9]+(-[a-z0-9]+)*$' AND
                        object_key = 'repos/' || repository_id ||
                            '/objects/sha256/' || checksum_sha256 AND
                        size_bytes BETWEEN 1 AND 1073741824 AND
                        (expected_reference_version IS NULL OR
                            expected_reference_version > 0) AND
                        state IN ('active', 'deleting', 'committed') AND
                        created_at_unix >= 0 AND
                        expires_at_unix > created_at_unix AND
                        expires_at_unix <= created_at_unix + 1800
                    )
                );

                CREATE INDEX idx_scope_cache_uploads_expiry
                    ON scope_cache_uploads (expires_at_unix, upload_id);
                CREATE UNIQUE INDEX idx_scope_cache_uploads_active_identity
                    ON scope_cache_uploads (repository_id, identity_digest)
                    WHERE state IN ('active', 'deleting');

                CREATE TABLE scope_cache_deletion_queue (
                    repository_id text NOT NULL,
                    checksum_sha256 varchar(64) NOT NULL,
                    not_before_unix bigint NOT NULL,
                    attempts integer NOT NULL,
                    last_error text,
                    PRIMARY KEY (repository_id, checksum_sha256),
                    CONSTRAINT fk_scope_cache_deletion_queue_object
                        FOREIGN KEY (repository_id, checksum_sha256)
                        REFERENCES scope_cache_objects (
                            repository_id, checksum_sha256
                        ) ON DELETE CASCADE,
                    CONSTRAINT scope_cache_deletion_queue_values CHECK (
                        checksum_sha256 ~ '^[0-9a-f]{64}$' AND
                        not_before_unix >= 0 AND
                        attempts >= 0 AND
                        (last_error IS NULL OR
                            char_length(last_error) BETWEEN 1 AND 8192)
                    )
                );

                CREATE INDEX idx_scope_cache_deletion_queue_due
                    ON scope_cache_deletion_queue (
                        not_before_unix, repository_id, checksum_sha256
                    );

                DROP TABLE scope_run_cache_objects;
                "#,
            )
            .await?;
        Ok(())
    }
}
