use super::{RunStore, entities};
use crate::error::PostgresError;
use scope_domain::runs::{
    job::RunJob,
    run::Run,
    runner::{Runner, RunnerGrant},
};
use sea_orm::{ColumnTrait, ConnectionTrait, EntityTrait, QueryFilter, QueryOrder, QuerySelect};
use std::collections::{BTreeMap, BTreeSet};

use super::StoredRunLog;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RepositoryRunner {
    pub runner: Runner,
    pub grant: RunnerGrant,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RunSnapshot {
    pub run: Run,
    pub jobs: Vec<RunJob>,
    pub logs_truncated: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RecentRunLogs {
    pub logs: Vec<StoredRunLog>,
    pub truncated_in_view: bool,
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

    pub async fn repository_runners(
        &self,
        repository_id: &str,
    ) -> Result<Vec<RepositoryRunner>, PostgresError> {
        let grant_models = entities::runner_grant::Entity::find()
            .filter(entities::runner_grant::Column::RepoId.eq(repository_id))
            .filter(entities::runner_grant::Column::RevokedAtUnix.is_null())
            .order_by_asc(entities::runner_grant::Column::Name)
            .all(self.db.as_ref())
            .await
            .map_err(PostgresError::internal)?;
        if grant_models.is_empty() {
            return Ok(Vec::new());
        }

        let runner_ids = grant_models
            .iter()
            .map(|grant| grant.runner_id.clone())
            .collect::<Vec<_>>();
        let mut runners = entities::runner::Entity::find()
            .filter(entities::runner::Column::Id.is_in(runner_ids))
            .all(self.db.as_ref())
            .await
            .map_err(PostgresError::internal)?
            .into_iter()
            .map(|model| {
                let id = model.id.clone();
                Ok((id, model.try_into_domain()?))
            })
            .collect::<Result<BTreeMap<_, _>, PostgresError>>()?;

        grant_models
            .into_iter()
            .map(|model| {
                let runner = runners.remove(&model.runner_id).ok_or_else(|| {
                    PostgresError::internal_message(
                        "active repository runner grant references a missing runner",
                    )
                })?;
                Ok(RepositoryRunner {
                    runner,
                    grant: model.try_into_domain()?,
                })
            })
            .collect()
    }

    pub async fn recent_run_logs(
        &self,
        run_id: &str,
        limit: u64,
    ) -> Result<RecentRunLogs, PostgresError> {
        let fetch_limit = limit.saturating_add(1);
        let mut logs = entities::run_log::Entity::find()
            .filter(entities::run_log::Column::RunId.eq(run_id))
            .order_by_desc(entities::run_log::Column::Position)
            .limit(fetch_limit)
            .all(self.db.as_ref())
            .await
            .map_err(PostgresError::internal)?;
        let truncated_in_view = logs.len() as u64 > limit;
        if truncated_in_view {
            logs.pop();
        }
        logs.reverse();
        let attempt_ids = logs
            .iter()
            .map(|model| model.attempt_id.clone())
            .collect::<Vec<_>>();
        let jobs_by_attempt = if attempt_ids.is_empty() {
            BTreeMap::new()
        } else {
            entities::run_attempt::Entity::find()
                .filter(entities::run_attempt::Column::Id.is_in(attempt_ids))
                .all(self.db.as_ref())
                .await
                .map_err(PostgresError::internal)?
                .into_iter()
                .map(|attempt| (attempt.id, attempt.job_key))
                .collect::<BTreeMap<_, _>>()
        };

        Ok(RecentRunLogs {
            truncated_in_view,
            logs: logs
                .into_iter()
                .map(|model| {
                    let position = entities::i64_to_u64(model.position, "run log position")?;
                    let run_id = model.run_id.clone();
                    let job_key =
                        jobs_by_attempt
                            .get(&model.attempt_id)
                            .cloned()
                            .ok_or_else(|| {
                                PostgresError::internal_message(
                                    "run log references a missing attempt",
                                )
                            })?;
                    Ok(StoredRunLog {
                        position,
                        run_id,
                        job_key,
                        chunk: model.try_into_domain()?,
                    })
                })
                .collect::<Result<_, PostgresError>>()?,
        })
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
            .filter(entities::run_attempt::Column::LogsTruncated.eq(true))
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
        .filter(entities::run_attempt::Column::LogsTruncated.eq(true))
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
