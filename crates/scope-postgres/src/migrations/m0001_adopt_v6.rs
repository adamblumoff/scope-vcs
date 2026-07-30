use sea_orm::{ConnectionTrait, DatabaseBackend, DbErr, Statement};
use sea_orm_migration::{MigrationName, MigrationTrait, SchemaManager};

const V6_SCHEMA: &str = include_str!("v6.sql");
const V6_SCHEMA_VERSION: i64 = 6;
const V6_COLUMN_FINGERPRINT: &str = "b8a8f1ef8a999a99deac8b9ad7477ce3";
const V6_CONSTRAINT_FINGERPRINT: &str = "2863e371d467aaa7ade9518b3579e097";
const V6_INDEX_FINGERPRINT: &str = "f68d7fcf133d87c6c62e0cabae5a7fb6";
const RETIRED_V6_TABLES: &[&str] = &[
    "scope_repository_git_clone_tokens",
    "scope_repository_git_snapshots",
    "scope_repository_settings",
    "scope_source_blob_cleanup_jobs",
];
const V6_TABLES: &[&str] = &[
    "scope_auth_identities",
    "scope_cli_browser_logins",
    "scope_cli_device_logins",
    "scope_cli_exchange_grants",
    "scope_cli_sessions",
    "scope_credit_ledger_entries",
    "scope_file_changes",
    "scope_git_heads",
    "scope_git_segments",
    "scope_live_files",
    "scope_logical_commits",
    "scope_metadata_locks",
    "scope_metadata_reset_events",
    "scope_metadata_schema",
    "scope_object_references",
    "scope_orphan_object_jobs",
    "scope_outbox_jobs",
    "scope_projection_files",
    "scope_projection_read_models",
    "scope_push_trigger_evaluations",
    "scope_repo_storage_cleanup_jobs",
    "scope_repositories",
    "scope_repository_first_push_tokens",
    "scope_repository_git_push_tokens",
    "scope_repository_invites",
    "scope_repository_members",
    "scope_request_change_blocks",
    "scope_request_discussion_read_states",
    "scope_request_discussion_replies",
    "scope_request_discussions",
    "scope_request_events",
    "scope_request_invitees",
    "scope_requests",
    "scope_run_attempts",
    "scope_run_logs",
    "scope_runner_grants",
    "scope_runners",
    "scope_runs",
    "scope_user_credit_accounts",
    "scope_users",
    "scope_visibility_events",
    "scope_workflow_revisions",
];

pub struct Migration;

impl MigrationName for Migration {
    fn name(&self) -> &str {
        "m0001_adopt_v6"
    }
}

#[sea_orm_migration::async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let db = manager.get_connection();
        let tables = scope_tables(db).await?;
        if tables.is_empty() {
            db.execute_unprepared(V6_SCHEMA).await?;
            return Ok(());
        }

        let expected_tables = V6_TABLES.join("\n");
        let actual_tables = tables.join("\n");
        let has_retired_tables = tables == v6_tables_with_retired();
        if actual_tables != expected_tables && !has_retired_tables {
            return Err(adoption_error("table set", expected_tables, actual_tables));
        }

        assert_v6_marker(db).await?;
        if has_retired_tables {
            db.execute_unprepared(
                "
                    DROP TABLE scope_repository_git_clone_tokens;
                    DROP TABLE scope_repository_git_snapshots;
                    DROP TABLE scope_repository_settings;
                    DROP TABLE scope_source_blob_cleanup_jobs;
                ",
            )
            .await?;
        }
        assert_fingerprint(
            db,
            "column",
            V6_COLUMN_FINGERPRINT,
            "
                SELECT md5(string_agg(
                    relation.relname || '|' || attribute.attnum || '|' ||
                    attribute.attname || '|' ||
                    format_type(attribute.atttypid, attribute.atttypmod) || '|' ||
                    attribute.attnotnull || '|' ||
                    coalesce(
                        pg_get_expr(default_value.adbin, default_value.adrelid),
                        ''
                    ) || '|' || attribute.attidentity::text || '|' ||
                    attribute.attgenerated::text,
                    E'\\n' ORDER BY relation.relname, attribute.attnum
                )) AS fingerprint
                FROM pg_attribute attribute
                JOIN pg_class relation ON relation.oid = attribute.attrelid
                JOIN pg_namespace namespace ON namespace.oid = relation.relnamespace
                LEFT JOIN pg_attrdef default_value
                    ON default_value.adrelid = attribute.attrelid
                    AND default_value.adnum = attribute.attnum
                WHERE namespace.nspname = current_schema()
                  AND relation.relkind = 'r'
                  AND left(relation.relname, 6) = 'scope_'
                  AND attribute.attnum > 0
                  AND NOT attribute.attisdropped
            ",
        )
        .await?;
        assert_fingerprint(
            db,
            "constraint",
            V6_CONSTRAINT_FINGERPRINT,
            "
                SELECT md5(string_agg(
                    relation.relname || '|' || constraint_row.conname || '|' ||
                    constraint_row.contype::text || '|' ||
                    replace(
                        pg_get_constraintdef(constraint_row.oid, true),
                        format('%I.', current_schema()),
                        ''
                    ),
                    E'\\n' ORDER BY relation.relname, constraint_row.conname
                )) AS fingerprint
                FROM pg_constraint constraint_row
                JOIN pg_class relation ON relation.oid = constraint_row.conrelid
                JOIN pg_namespace namespace ON namespace.oid = relation.relnamespace
                WHERE namespace.nspname = current_schema()
                  AND left(relation.relname, 6) = 'scope_'
            ",
        )
        .await?;
        assert_fingerprint(
            db,
            "index",
            V6_INDEX_FINGERPRINT,
            "
                SELECT md5(string_agg(
                    table_relation.relname || '|' || index_relation.relname || '|' ||
                    replace(
                        pg_get_indexdef(index_relation.oid),
                        format('%I.', current_schema()),
                        ''
                    ),
                    E'\\n' ORDER BY table_relation.relname, index_relation.relname
                )) AS fingerprint
                FROM pg_index index_row
                JOIN pg_class table_relation ON table_relation.oid = index_row.indrelid
                JOIN pg_class index_relation ON index_relation.oid = index_row.indexrelid
                JOIN pg_namespace namespace ON namespace.oid = table_relation.relnamespace
                WHERE namespace.nspname = current_schema()
                  AND left(table_relation.relname, 6) = 'scope_'
            ",
        )
        .await
    }
}

fn v6_tables_with_retired() -> Vec<String> {
    let mut tables = V6_TABLES
        .iter()
        .chain(RETIRED_V6_TABLES)
        .map(|table| (*table).to_string())
        .collect::<Vec<_>>();
    tables.sort();
    tables
}

async fn scope_tables<C>(db: &C) -> Result<Vec<String>, DbErr>
where
    C: ConnectionTrait,
{
    db.query_all(Statement::from_string(
        DatabaseBackend::Postgres,
        "
            SELECT tablename
            FROM pg_tables
            WHERE schemaname = current_schema()
              AND left(tablename, 6) = 'scope_'
            ORDER BY tablename
        "
        .to_string(),
    ))
    .await?
    .into_iter()
    .map(|row| row.try_get::<String>("", "tablename"))
    .collect()
}

async fn assert_v6_marker<C>(db: &C) -> Result<(), DbErr>
where
    C: ConnectionTrait,
{
    let markers = db
        .query_all(Statement::from_string(
            DatabaseBackend::Postgres,
            "SELECT key, version, ready FROM scope_metadata_schema ORDER BY key".to_string(),
        ))
        .await?;
    if markers.len() != 1 {
        return Err(DbErr::Migration(format!(
            "cannot adopt Scope metadata schema: expected one v6 marker, found {}",
            markers.len()
        )));
    }
    let marker = &markers[0];
    let key = marker.try_get::<String>("", "key")?;
    let version = marker.try_get::<i64>("", "version")?;
    let ready = marker.try_get::<bool>("", "ready")?;
    if key != "current" || version != V6_SCHEMA_VERSION || !ready {
        return Err(DbErr::Migration(format!(
            "cannot adopt Scope metadata schema: expected current ready v{V6_SCHEMA_VERSION}, found key={key} version={version} ready={ready}"
        )));
    }
    Ok(())
}

async fn assert_fingerprint<C>(db: &C, kind: &str, expected: &str, query: &str) -> Result<(), DbErr>
where
    C: ConnectionTrait,
{
    let actual = db
        .query_one(Statement::from_string(
            DatabaseBackend::Postgres,
            query.to_string(),
        ))
        .await?
        .ok_or_else(|| DbErr::Migration(format!("PostgreSQL did not report the v6 {kind} set")))?
        .try_get::<String>("", "fingerprint")?;
    if actual != expected {
        return Err(adoption_error(
            &format!("{kind} fingerprint"),
            expected,
            actual,
        ));
    }
    Ok(())
}

fn adoption_error(kind: &str, expected: impl AsRef<str>, actual: impl AsRef<str>) -> DbErr {
    DbErr::Migration(format!(
        "cannot adopt Scope metadata schema: expected v6 {kind} [{}], found [{}]",
        expected.as_ref().replace('\n', ", "),
        actual.as_ref().replace('\n', ", ")
    ))
}
