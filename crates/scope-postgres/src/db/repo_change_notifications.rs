use super::RepositoryStore;
use crate::error::PostgresError;
use sea_orm::{ConnectionTrait, DbBackend, Statement};
use std::sync::Arc;

const POSTGRES_REPO_CHANGE_CHANNEL: &str = "scope_repo_changes";

impl RepositoryStore {
    pub fn start_repo_change_listener(
        &self,
        on_payload: impl Fn(String) + Send + Sync + 'static,
    ) -> anyhow::Result<()> {
        super::postgres_notifications::start_listener(
            self.postgres_database_url.as_ref(),
            "scope-repo-change-listener",
            POSTGRES_REPO_CHANGE_CHANNEL,
            on_payload,
        )
    }

    pub async fn notify_repo_change(&self, payload: &str) -> Result<(), PostgresError> {
        let db = Arc::clone(&self.db);
        db.execute(Statement::from_sql_and_values(
            DbBackend::Postgres,
            format!("SELECT pg_notify('{POSTGRES_REPO_CHANGE_CHANNEL}', $1)"),
            [payload.into()],
        ))
        .await
        .map_err(PostgresError::internal)?;
        Ok(())
    }
}
