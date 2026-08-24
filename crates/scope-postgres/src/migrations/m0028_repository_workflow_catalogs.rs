use sea_orm::ConnectionTrait;
use sea_orm_migration::{DbErr, MigrationName, MigrationTrait, SchemaManager};

pub struct Migration;

impl MigrationName for Migration {
    fn name(&self) -> &str {
        "m0028_repository_workflow_catalogs"
    }
}

#[sea_orm_migration::async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .get_connection()
            .execute_unprepared(
                r#"
                CREATE TABLE scope_repository_workflow_catalogs (
                    repo_id text PRIMARY KEY
                        REFERENCES scope_repositories(id) ON DELETE CASCADE,
                    source_head_oid text NOT NULL,
                    source_change_version bigint NOT NULL,
                    configuration_error text,
                    CONSTRAINT scope_repository_workflow_catalog_values CHECK (
                        char_length(source_head_oid) = 40 AND
                        source_head_oid = lower(source_head_oid) AND
                        source_head_oid ~ '^[0-9a-f]{40}$' AND
                        source_change_version > 0 AND
                        (
                            configuration_error IS NULL OR
                            octet_length(configuration_error) BETWEEN 1 AND 4096
                        )
                    )
                );

                CREATE TABLE scope_repository_workflow_files (
                    repo_id text NOT NULL
                        REFERENCES scope_repository_workflow_catalogs(repo_id)
                        ON DELETE CASCADE,
                    path text NOT NULL,
                    oid text NOT NULL,
                    size_bytes bigint NOT NULL,
                    git_file_mode text NOT NULL,
                    content_bytes bytea NOT NULL,
                    PRIMARY KEY (repo_id, path),
                    CONSTRAINT scope_repository_workflow_file_values CHECK (
                        path ~ '^/\.scope/runs/[a-z0-9]+(-[a-z0-9]+)*\.ya?ml$' AND
                        octet_length(
                            regexp_replace(
                                regexp_replace(path, '^/\.scope/runs/', ''),
                                '\.ya?ml$',
                                ''
                            )
                        ) BETWEEN 1 AND 64 AND
                        char_length(oid) = 40 AND
                        oid = lower(oid) AND
                        oid ~ '^[0-9a-f]{40}$' AND
                        size_bytes BETWEEN 0 AND 65536 AND
                        octet_length(content_bytes) = size_bytes AND
                        git_file_mode IN ('100644', '100755')
                    )
                );

                CREATE FUNCTION scope_check_repository_workflow_file()
                RETURNS trigger
                LANGUAGE plpgsql
                AS $$
                BEGIN
                    IF EXISTS (
                        SELECT 1
                        FROM scope_repository_workflow_catalogs
                        WHERE repo_id = NEW.repo_id
                          AND configuration_error IS NOT NULL
                    ) THEN
                        RAISE EXCEPTION 'rejected repository workflow catalog cannot contain files';
                    END IF;

                    IF TG_OP = 'INSERT' OR
                       NEW.repo_id <> OLD.repo_id OR
                       NEW.path <> OLD.path THEN
                        IF (
                            SELECT count(*)
                            FROM scope_repository_workflow_files
                            WHERE repo_id = NEW.repo_id
                        ) >= 64 THEN
                            RAISE EXCEPTION 'repository workflow catalog cannot contain more than 64 files';
                        END IF;
                    END IF;

                    RETURN NEW;
                END;
                $$;

                CREATE TRIGGER scope_repository_workflow_file_guard
                BEFORE INSERT OR UPDATE ON scope_repository_workflow_files
                FOR EACH ROW
                EXECUTE FUNCTION scope_check_repository_workflow_file();

                CREATE FUNCTION scope_check_repository_workflow_catalog_rejection()
                RETURNS trigger
                LANGUAGE plpgsql
                AS $$
                BEGIN
                    IF NEW.configuration_error IS NOT NULL AND EXISTS (
                        SELECT 1
                        FROM scope_repository_workflow_files
                        WHERE repo_id = NEW.repo_id
                    ) THEN
                        RAISE EXCEPTION 'rejected repository workflow catalog cannot contain files';
                    END IF;

                    RETURN NEW;
                END;
                $$;

                CREATE TRIGGER scope_repository_workflow_catalog_rejection_guard
                BEFORE UPDATE OF configuration_error
                ON scope_repository_workflow_catalogs
                FOR EACH ROW
                EXECUTE FUNCTION scope_check_repository_workflow_catalog_rejection();
                "#,
            )
            .await?;
        Ok(())
    }
}
