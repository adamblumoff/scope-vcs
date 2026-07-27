use super::RepositoryStore;
use crate::error::PostgresError;
use sea_orm::{ConnectionTrait, DbBackend, Statement};
use std::{sync::Arc, time::Duration};

const POSTGRES_REPO_CHANGE_CHANNEL: &str = "scope_repo_changes";
const RECONNECT_DELAY: Duration = Duration::from_secs(2);
type PayloadHandler = Arc<dyn Fn(String) + Send + Sync>;

impl RepositoryStore {
    pub fn start_repo_change_listener(
        &self,
        on_payload: impl Fn(String) + Send + Sync + 'static,
    ) -> anyhow::Result<()> {
        let Some(database_url) = &self.postgres_database_url else {
            return Ok(());
        };
        let database_url = database_url.to_string();
        let on_payload: PayloadHandler = Arc::new(on_payload);
        std::thread::Builder::new()
            .name("scope-repo-change-listener".to_string())
            .spawn(move || {
                let runtime = match tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                {
                    Ok(runtime) => runtime,
                    Err(error) => {
                        tracing::error!(%error, "failed to start repo change listener runtime");
                        return;
                    }
                };
                runtime.block_on(async move {
                    loop {
                        if let Err(error) =
                            listen_for_repo_changes(&database_url, Arc::clone(&on_payload)).await
                        {
                            tracing::warn!(%error, "repo change listener disconnected");
                            tokio::time::sleep(RECONNECT_DELAY).await;
                        }
                    }
                });
            })?;
        Ok(())
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

async fn listen_for_repo_changes(
    database_url: &str,
    on_payload: PayloadHandler,
) -> Result<(), sqlx::Error> {
    let mut listener = sqlx::postgres::PgListener::connect(database_url).await?;
    listener.listen(POSTGRES_REPO_CHANGE_CHANNEL).await?;
    loop {
        let notification = listener.recv().await?;
        on_payload(notification.payload().to_string());
    }
}
