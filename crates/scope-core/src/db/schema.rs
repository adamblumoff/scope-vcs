use sea_orm::{ConnectionTrait, DatabaseConnection, DbErr, Statement};

const METADATA_SCHEMA: &str = include_str!("schema.sql");
const RESET_TABLES: &str = "
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

pub async fn migrate_metadata_schema(db: &DatabaseConnection) -> Result<(), DbErr> {
    reset_metadata_schema(db).await?;
    db.execute_unprepared(METADATA_SCHEMA).await?;
    Ok(())
}

pub async fn assert_metadata_schema_ready(db: &DatabaseConnection) -> Result<(), DbErr> {
    let lock_row = db
        .query_one(Statement::from_string(
            db.get_database_backend(),
            "SELECT 1 FROM scope_metadata_locks LIMIT 1".to_string(),
        ))
        .await?;
    if lock_row.is_none() {
        return Err(DbErr::Custom(
            "Scope metadata lock row is not ready for workers".to_string(),
        ));
    }
    Ok(())
}

pub async fn reset_metadata_schema(db: &DatabaseConnection) -> Result<(), DbErr> {
    db.execute_unprepared(&format!("DROP TABLE IF EXISTS {RESET_TABLES} CASCADE"))
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
}
