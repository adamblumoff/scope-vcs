use super::{MetadataStore, MetadataStoreInner, run_api_db_on};
use crate::{
    error::ApiError,
    repo_events::{
        POSTGRES_REPO_CHANGE_CHANNEL, RepoChangeBus, RepoChangeEvent, RepoChangeNotification,
    },
};
use sea_orm::{ConnectionTrait, DbBackend, Statement};
use std::sync::Arc;

impl MetadataStore {
    pub(crate) fn start_repo_change_listener(&self, bus: RepoChangeBus) -> anyhow::Result<()> {
        let Some(database_url) = &self.postgres_database_url else {
            return Ok(());
        };
        bus.start_postgres_listener(database_url.to_string())
    }

    pub(crate) fn notify_repo_change(
        &self,
        origin_id: &str,
        event: &RepoChangeEvent,
    ) -> Result<(), ApiError> {
        let notification = RepoChangeNotification::new(origin_id, event);
        let payload = serde_json::to_string(&notification).map_err(ApiError::internal)?;
        match self.inner.as_ref() {
            MetadataStoreInner::Postgres { db, runtime } => {
                let db = Arc::clone(db);
                run_api_db_on(runtime, async move {
                    db.execute(Statement::from_sql_and_values(
                        DbBackend::Postgres,
                        format!("SELECT pg_notify('{POSTGRES_REPO_CHANGE_CHANNEL}', $1)"),
                        [payload.into()],
                    ))
                    .await
                    .map_err(ApiError::internal)?;
                    Ok(())
                })
            }
            #[cfg(any(test, feature = "memory-metadata"))]
            MetadataStoreInner::Memory(_) => Ok(()),
        }
    }
}
