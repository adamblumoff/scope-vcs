use super::{RepositoryStore, entities};
use crate::error::PostgresError;
use scope_domain::content_ref::ContentRef;
use sea_orm::{ConnectionTrait, DatabaseBackend, DatabaseConnection, EntityTrait, Statement};
use sqlx::{Connection as _, PgConnection};
use std::future::Future;

pub struct ContentRefFence {
    connection: Option<PgConnection>,
    keys: Vec<i64>,
}

impl ContentRefFence {
    pub async fn release(mut self) {
        if self.connection.is_none() {
            return;
        }
        for key in self.keys.iter().rev() {
            let connection = self
                .connection
                .as_mut()
                .expect("content fence connection is present until release completes");
            if let Err(error) = sqlx::query("SELECT pg_advisory_unlock($1)")
                .bind(key)
                .execute(&mut *connection)
                .await
            {
                tracing::warn!(
                    error = %error,
                    "failed to explicitly release content fence; dropping its Postgres session"
                );
                return;
            }
        }
        self.connection.take();
    }
}

impl Drop for ContentRefFence {
    fn drop(&mut self) {
        self.connection.take();
    }
}

impl RepositoryStore {
    /// Serializes filesystem deletion and repository creation for one stable owner/name path.
    /// The session lock spans external I/O without holding a metadata transaction open.
    pub async fn with_repo_storage_lock<R, F, Fut, E>(&self, repo_id: &str, op: F) -> Result<R, E>
    where
        F: FnOnce() -> Fut,
        Fut: Future<Output = Result<R, E>>,
        E: From<PostgresError>,
    {
        let schema = current_schema(self.db.as_ref()).await?;
        let connection =
            dedicated_fence_connection(self.postgres_database_url.as_deref(), &schema).await?;
        let lock = sea_orm::sqlx::postgres::PgAdvisoryLock::new(format!(
            "scope:repo-storage:{schema}:{repo_id}"
        ));
        let guard = lock
            .acquire(connection)
            .await
            .map_err(PostgresError::internal)?;
        let result = op().await;
        if let Err(error) = guard.release_now().await {
            tracing::warn!(
                error = %error,
                "failed to explicitly release repository storage fence; dropping its Postgres session"
            );
        }
        result
    }

    pub async fn repository_exists(&self, repo_id: &str) -> Result<bool, PostgresError> {
        entities::repository::Entity::find_by_id(repo_id.to_string())
            .one(self.db.as_ref())
            .await
            .map(|row| row.is_some())
            .map_err(PostgresError::internal)
    }
}

pub(super) async fn acquire_content_ref_fence(
    db: &DatabaseConnection,
    postgres_database_url: Option<&str>,
    content_refs: &[ContentRef],
) -> Result<ContentRefFence, PostgresError> {
    let schema = current_schema(db).await?;
    let mut encoded = content_refs
        .iter()
        .map(serde_json::to_string)
        .collect::<Result<Vec<_>, _>>()
        .map_err(PostgresError::internal)?;
    encoded.sort_unstable();
    encoded.dedup();

    let keys = encoded
        .into_iter()
        .map(|content_ref| {
            sea_orm::sqlx::postgres::PgAdvisoryLock::new(format!(
                "scope:content-ref:{schema}:{content_ref}"
            ))
            .key()
            .as_bigint()
            .expect("string-derived advisory locks use one bigint key")
        })
        .collect::<Vec<_>>();
    let connection = dedicated_fence_connection(postgres_database_url, &schema).await?;
    let mut fence = ContentRefFence {
        connection: Some(connection),
        keys,
    };
    for key in &fence.keys {
        let connection = fence
            .connection
            .as_mut()
            .expect("content fence connection is present while acquiring locks");
        if let Err(error) = sqlx::query("SELECT pg_advisory_lock($1)")
            .bind(key)
            .execute(&mut *connection)
            .await
        {
            return Err(PostgresError::internal(error));
        }
    }
    Ok(fence)
}

async fn dedicated_fence_connection(
    postgres_database_url: Option<&str>,
    schema: &str,
) -> Result<PgConnection, PostgresError> {
    let database_url = postgres_database_url.ok_or_else(|| {
        PostgresError::internal_message(
            "Postgres database URL is unavailable for external-I/O fence",
        )
    })?;
    let mut connection = PgConnection::connect(database_url)
        .await
        .map_err(PostgresError::internal)?;
    sqlx::query("SELECT set_config('search_path', $1, false)")
        .bind(schema)
        .execute(&mut connection)
        .await
        .map_err(PostgresError::internal)?;
    Ok(connection)
}

async fn current_schema(db: &DatabaseConnection) -> Result<String, PostgresError> {
    db.query_one(Statement::from_string(
        DatabaseBackend::Postgres,
        "SELECT current_schema() AS schema".to_string(),
    ))
    .await
    .map_err(PostgresError::internal)?
    .ok_or_else(|| PostgresError::internal_message("Postgres did not return its schema"))?
    .try_get::<String>("", "schema")
    .map_err(PostgresError::internal)
}
