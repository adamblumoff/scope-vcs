mod ledger;
mod references;
mod retirement;
mod spans;

pub(super) use references::{insert_git_segment_references, release_git_segment_references};
pub(super) use retirement::retire_git_segment;
pub(super) use spans::{load_git_pack_spans, publish_git_segment};

use super::{RepositoryStore, content_fences};
use crate::error::PostgresError;
use sqlx::PgConnection;

pub struct RepositoryGitWriteLease {
    connection: Option<PgConnection>,
    key: i64,
}

impl RepositoryGitWriteLease {
    pub async fn release(mut self) {
        let Some(mut connection) = self.connection.take() else {
            return;
        };
        if let Err(error) = sqlx::query("SELECT pg_advisory_unlock($1)")
            .bind(self.key)
            .execute(&mut connection)
            .await
        {
            tracing::warn!(
                error = %error,
                "failed to release repository Git write lease; dropping its Postgres session"
            );
        }
    }
}

impl Drop for RepositoryGitWriteLease {
    fn drop(&mut self) {
        self.connection.take();
    }
}

impl RepositoryStore {
    pub async fn acquire_git_write_lease(
        &self,
        repo_id: &str,
    ) -> Result<RepositoryGitWriteLease, PostgresError> {
        if repo_id.trim().is_empty() {
            return Err(PostgresError::internal_message(
                "repository Git write lease requires a repository id",
            ));
        }
        let schema = content_fences::current_schema(self.db.as_ref()).await?;
        let mut connection = content_fences::dedicated_fence_connection(
            self.postgres_database_url.as_deref(),
            &schema,
        )
        .await?;
        let key = sea_orm::sqlx::postgres::PgAdvisoryLock::new(format!(
            "scope:git-write:{schema}:{repo_id}"
        ))
        .key()
        .as_bigint()
        .expect("string-derived advisory locks use one bigint key");
        sqlx::query("SELECT pg_advisory_lock($1)")
            .bind(key)
            .execute(&mut connection)
            .await
            .map_err(PostgresError::internal)?;
        Ok(RepositoryGitWriteLease {
            connection: Some(connection),
            key,
        })
    }
}

#[cfg(test)]
mod tests;
