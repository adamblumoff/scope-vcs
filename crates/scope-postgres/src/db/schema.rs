use sea_orm::{
    ConnectionTrait, DatabaseBackend, DatabaseConnection, DbErr, Statement, TransactionTrait,
};

const METADATA_SCHEMA: &str = include_str!("schema.sql");
const METADATA_SCHEMA_KEY: &str = "current";
// Increment whenever schema.sql changes incompatibly; pre-alpha upgrades intentionally reset data.
pub const METADATA_SCHEMA_VERSION: i64 = 5;
const RESET_TABLES: &str = "
    scope_metadata_schema,
    scope_run_logs,
    scope_run_attempts,
    scope_runs,
    scope_workflow_revisions,
    scope_runner_grants,
    scope_runners,
    scope_repository_invites,
    scope_repository_members,
    scope_live_files,
    scope_object_references,
    scope_file_changes,
    scope_visibility_events,
    scope_logical_commits,
    scope_projection_files,
    scope_projection_read_models,
    scope_credit_ledger_entries,
    scope_request_discussion_read_states,
    scope_request_discussion_replies,
    scope_request_discussions,
    scope_request_invitees,
    scope_request_change_blocks,
    scope_request_events,
    scope_requests,
    scope_user_credit_accounts,
    scope_outbox_jobs,
    scope_git_segments,
    scope_git_heads,
    scope_repository_git_push_tokens,
    scope_repository_first_push_tokens,
    scope_repositories,
    scope_cli_sessions,
    scope_cli_exchange_grants,
    scope_cli_browser_logins,
    scope_cli_device_logins,
    scope_auth_identities,
    scope_users,
    scope_orphan_object_jobs,
    scope_repo_storage_cleanup_jobs,
    scope_metadata_locks,
    scope_metadata_reset_events
";

pub async fn initialize_metadata_schema(
    db: &DatabaseConnection,
    deploy_revision: &str,
) -> Result<(), DbErr> {
    prepare_metadata_schema(db, deploy_revision, false).await
}

pub async fn recreate_metadata_schema(
    db: &DatabaseConnection,
    deploy_revision: &str,
) -> Result<(), DbErr> {
    prepare_metadata_schema(db, deploy_revision, true).await
}

async fn prepare_metadata_schema(
    db: &DatabaseConnection,
    deploy_revision: &str,
    force_reset: bool,
) -> Result<(), DbErr> {
    let tx = db.begin().await?;
    tx.execute_unprepared(
        "SELECT pg_advisory_xact_lock(hashtextextended('scope:metadata-schema', 0))",
    )
    .await?;

    let current_version = current_schema_version(&tx).await?;
    if force_reset || current_version != Some(METADATA_SCHEMA_VERSION) {
        reset_metadata_schema(&tx).await?;
        tx.execute_unprepared(METADATA_SCHEMA).await?;
    }
    tx.execute_unprepared(
        "INSERT INTO scope_metadata_locks (key) VALUES ('catalog') ON CONFLICT (key) DO NOTHING",
    )
    .await?;
    mark_metadata_schema_ready(&tx, deploy_revision).await?;
    tx.commit().await?;
    Ok(())
}

pub async fn assert_metadata_schema_ready<C>(
    conn: &C,
    expected_deploy_revision: &str,
) -> Result<(), DbErr>
where
    C: ConnectionTrait,
{
    let marker = conn
        .query_one(Statement::from_sql_and_values(
            DatabaseBackend::Postgres,
            "
                SELECT version, deploy_revision, ready
                FROM scope_metadata_schema
                WHERE key = $1
            ",
            [METADATA_SCHEMA_KEY.into()],
        ))
        .await?;
    let Some(marker) = marker else {
        return Err(DbErr::Custom(
            "Scope metadata schema marker is missing".to_string(),
        ));
    };
    let version = marker.try_get::<i64>("", "version")?;
    let deploy_revision = marker.try_get::<String>("", "deploy_revision")?;
    let ready = marker.try_get::<bool>("", "ready")?;
    if version != METADATA_SCHEMA_VERSION {
        return Err(DbErr::Custom(format!(
            "Scope metadata schema version {version} does not match expected version {METADATA_SCHEMA_VERSION}"
        )));
    }
    if deploy_revision != expected_deploy_revision {
        return Err(DbErr::Custom(format!(
            "Scope metadata deploy revision {deploy_revision} does not match worker revision {expected_deploy_revision}"
        )));
    }
    if !ready {
        return Err(DbErr::Custom(
            "Scope metadata schema is not ready".to_string(),
        ));
    }
    Ok(())
}

pub async fn reset_metadata_schema<C>(conn: &C) -> Result<(), DbErr>
where
    C: ConnectionTrait,
{
    conn.execute_unprepared(&format!("DROP TABLE IF EXISTS {RESET_TABLES} CASCADE"))
        .await?;
    Ok(())
}

async fn current_schema_version<C>(conn: &C) -> Result<Option<i64>, DbErr>
where
    C: ConnectionTrait,
{
    let table_exists = conn
        .query_one(Statement::from_string(
            DatabaseBackend::Postgres,
            "SELECT to_regclass('scope_metadata_schema') IS NOT NULL AS exists".to_string(),
        ))
        .await?
        .ok_or_else(|| DbErr::Custom("Postgres did not report schema marker state".to_string()))?
        .try_get::<bool>("", "exists")?;
    if !table_exists {
        return Ok(None);
    }
    conn.query_one(Statement::from_sql_and_values(
        DatabaseBackend::Postgres,
        "SELECT version FROM scope_metadata_schema WHERE key = $1 AND ready = TRUE",
        [METADATA_SCHEMA_KEY.into()],
    ))
    .await?
    .map(|row| row.try_get::<i64>("", "version"))
    .transpose()
}

async fn mark_metadata_schema_ready<C>(conn: &C, deploy_revision: &str) -> Result<(), DbErr>
where
    C: ConnectionTrait,
{
    conn.execute(Statement::from_sql_and_values(
        DatabaseBackend::Postgres,
        "
            INSERT INTO scope_metadata_schema (key, version, deploy_revision, ready)
            VALUES ($1, $2, $3, TRUE)
            ON CONFLICT (key) DO UPDATE
            SET version = EXCLUDED.version,
                deploy_revision = EXCLUDED.deploy_revision,
                ready = EXCLUDED.ready
        ",
        [
            METADATA_SCHEMA_KEY.into(),
            METADATA_SCHEMA_VERSION.into(),
            deploy_revision.into(),
        ],
    ))
    .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::{MetadataStore, TestDatabaseTarget};
    use std::collections::BTreeSet;

    #[tokio::test]
    async fn reset_list_matches_owned_schema_tables() {
        let store =
            MetadataStore::connect_fresh_for_tests(&TestDatabaseTarget::required().unwrap())
                .unwrap();
        let actual = store
            .db
            .query_all(Statement::from_string(
                store.db.get_database_backend(),
                "SELECT tablename FROM pg_tables WHERE schemaname = current_schema()".to_string(),
            ))
            .await
            .unwrap()
            .into_iter()
            .map(|row| row.try_get::<String>("", "tablename").unwrap())
            .collect::<BTreeSet<_>>();
        let expected = RESET_TABLES
            .split(',')
            .map(str::trim)
            .filter(|name| !name.is_empty())
            .map(str::to_string)
            .collect::<BTreeSet<_>>();

        assert_eq!(actual, expected);
    }

    #[tokio::test]
    async fn matching_schema_version_preserves_data_while_advancing_release_revision() {
        let store =
            MetadataStore::connect_fresh_for_tests(&TestDatabaseTarget::required().unwrap())
                .unwrap();
        store
            .db
            .execute_unprepared("INSERT INTO scope_metadata_locks (key) VALUES ('sentinel')")
            .await
            .unwrap();

        initialize_metadata_schema(store.db.as_ref(), "next-release")
            .await
            .unwrap();

        assert_metadata_schema_ready(store.db.as_ref(), "next-release")
            .await
            .unwrap();
        let sentinel = store
            .db
            .query_one(Statement::from_string(
                DatabaseBackend::Postgres,
                "SELECT 1 FROM scope_metadata_locks WHERE key = 'sentinel'".to_string(),
            ))
            .await
            .unwrap();
        assert!(sentinel.is_some());
    }

    #[tokio::test]
    async fn mismatched_schema_version_forces_destructive_reset() {
        let store =
            MetadataStore::connect_fresh_for_tests(&TestDatabaseTarget::required().unwrap())
                .unwrap();
        store
            .db
            .execute_unprepared("UPDATE scope_metadata_schema SET version = 0")
            .await
            .unwrap();
        store
            .db
            .execute_unprepared("INSERT INTO scope_metadata_locks (key) VALUES ('sentinel')")
            .await
            .unwrap();

        initialize_metadata_schema(store.db.as_ref(), "next-release")
            .await
            .unwrap();

        let sentinel = store
            .db
            .query_one(Statement::from_string(
                DatabaseBackend::Postgres,
                "SELECT 1 FROM scope_metadata_locks WHERE key = 'sentinel'".to_string(),
            ))
            .await
            .unwrap();
        assert!(sentinel.is_none());
    }

    #[tokio::test]
    async fn schema_marker_requires_matching_deploy_revision() {
        let store =
            MetadataStore::connect_fresh_for_tests(&TestDatabaseTarget::required().unwrap())
                .unwrap();

        assert_metadata_schema_ready(store.db.as_ref(), "local")
            .await
            .unwrap();
        let error = assert_metadata_schema_ready(store.db.as_ref(), "other")
            .await
            .unwrap_err();

        assert!(error.to_string().contains("does not match worker revision"));
    }
}
