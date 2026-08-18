use super::{RunStore, entities};
use crate::error::PostgresError;
use scope_domain::runs::{job::RunJob, run::Run};
use sea_orm::{ColumnTrait, ConnectionTrait, EntityTrait, QueryFilter, QueryOrder, QuerySelect};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RunSnapshot {
    pub run: Run,
    pub jobs: Vec<RunJob>,
    pub logs_truncated: bool,
}

impl RunStore {
    pub async fn run_snapshot(&self, run_id: &str) -> Result<Option<RunSnapshot>, PostgresError> {
        let tx = super::begin_metadata_read_snapshot(self.db.as_ref()).await?;
        let Some(run) = entities::run::Entity::find_by_id(run_id.to_string())
            .one(&tx)
            .await
            .map_err(PostgresError::internal)?
            .map(entities::run::Model::try_into_domain)
            .transpose()?
        else {
            tx.commit().await.map_err(PostgresError::internal)?;
            return Ok(None);
        };
        let mut jobs = run_jobs_by_ids(&tx, &[run_id.to_string()]).await?;
        let jobs = jobs
            .remove(run_id)
            .filter(|jobs| !jobs.is_empty())
            .ok_or_else(|| PostgresError::internal_message("run is missing its persisted jobs"))?;
        let logs_truncated = run_has_truncated_logs_with(&tx, run_id).await?;
        tx.commit().await.map_err(PostgresError::internal)?;
        Ok(Some(RunSnapshot {
            run,
            jobs,
            logs_truncated,
        }))
    }

    pub async fn run_jobs(&self, run_id: &str) -> Result<Vec<RunJob>, PostgresError> {
        let mut jobs = run_jobs_by_ids(self.db.as_ref(), &[run_id.to_string()]).await?;
        jobs.remove(run_id)
            .filter(|jobs| !jobs.is_empty())
            .ok_or_else(|| PostgresError::internal_message("run is missing its persisted jobs"))
    }

    pub async fn run_jobs_by_ids(
        &self,
        run_ids: &[String],
    ) -> Result<BTreeMap<String, Vec<RunJob>>, PostgresError> {
        run_jobs_by_ids(self.db.as_ref(), run_ids).await
    }

    pub async fn run_has_truncated_logs(&self, run_id: &str) -> Result<bool, PostgresError> {
        run_has_truncated_logs_with(self.db.as_ref(), run_id).await
    }

    pub async fn run_ids_with_truncated_logs(
        &self,
        run_ids: &[String],
    ) -> Result<BTreeSet<String>, PostgresError> {
        if run_ids.is_empty() {
            return Ok(BTreeSet::new());
        }
        entities::run_attempt::Entity::find()
            .select_only()
            .column(entities::run_attempt::Column::RunId)
            .filter(entities::run_attempt::Column::RunId.is_in(run_ids.to_vec()))
            .filter(entities::run_attempt::Column::FirstTruncatedStepIndex.is_not_null())
            .into_tuple::<String>()
            .all(self.db.as_ref())
            .await
            .map_err(PostgresError::internal)
            .map(|ids| ids.into_iter().collect())
    }
}

async fn run_has_truncated_logs_with<C>(conn: &C, run_id: &str) -> Result<bool, PostgresError>
where
    C: ConnectionTrait,
{
    Ok(entities::run_attempt::Entity::find()
        .filter(entities::run_attempt::Column::RunId.eq(run_id))
        .filter(entities::run_attempt::Column::FirstTruncatedStepIndex.is_not_null())
        .one(conn)
        .await
        .map_err(PostgresError::internal)?
        .is_some())
}

pub(super) async fn run_jobs_by_ids<C>(
    conn: &C,
    run_ids: &[String],
) -> Result<BTreeMap<String, Vec<RunJob>>, PostgresError>
where
    C: ConnectionTrait,
{
    if run_ids.is_empty() {
        return Ok(BTreeMap::new());
    }
    Ok(entities::run_job::Entity::find()
        .filter(entities::run_job::Column::RunId.is_in(run_ids.to_vec()))
        .order_by_asc(entities::run_job::Column::RunId)
        .order_by_asc(entities::run_job::Column::JobKey)
        .all(conn)
        .await
        .map_err(PostgresError::internal)?
        .into_iter()
        .map(entities::run_job::Model::try_into_domain)
        .collect::<Result<Vec<_>, _>>()?
        .into_iter()
        .fold(BTreeMap::new(), |mut jobs, job| {
            jobs.entry(job.run_id.clone()).or_default().push(job);
            jobs
        }))
}
