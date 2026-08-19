use super::{RunStore, entities};
use crate::error::PostgresError;
use scope_domain::error::DomainErrorKind;
use sea_orm::EntityTrait;

impl RunStore {
    pub async fn authorize_cache_grant(
        &self,
        attempt_id: &str,
        repository_id: &str,
        now_unix: u64,
    ) -> Result<bool, PostgresError> {
        let tx = super::begin_metadata_read_snapshot(self.db.as_ref()).await?;
        let Some(attempt_record) =
            entities::run_attempt::Entity::find_by_id(attempt_id.to_string())
                .one(&tx)
                .await
                .map_err(PostgresError::internal)?
        else {
            tx.commit().await.map_err(PostgresError::internal)?;
            return Ok(false);
        };
        let Some(run_record) = entities::run::Entity::find_by_id(attempt_record.run_id.clone())
            .one(&tx)
            .await
            .map_err(PostgresError::internal)?
        else {
            tx.commit().await.map_err(PostgresError::internal)?;
            return Ok(false);
        };
        if run_record.repo_id != repository_id {
            tx.commit().await.map_err(PostgresError::internal)?;
            return Ok(false);
        }
        let Some(job_record) = entities::run_job::Entity::find_by_id((
            attempt_record.run_id.clone(),
            attempt_record.job_key.clone(),
        ))
        .one(&tx)
        .await
        .map_err(PostgresError::internal)?
        else {
            tx.commit().await.map_err(PostgresError::internal)?;
            return Ok(false);
        };
        let attempt = attempt_record.try_into_domain()?;
        let job = job_record.try_into_domain()?;
        let authorized = match attempt.authorize_cache_access(&job, now_unix) {
            Ok(()) => true,
            Err(error)
                if matches!(
                    error.kind,
                    DomainErrorKind::AuthenticationFailed | DomainErrorKind::Conflict
                ) =>
            {
                false
            }
            Err(error) => return Err(error.into()),
        };
        tx.commit().await.map_err(PostgresError::internal)?;
        Ok(authorized)
    }
}
