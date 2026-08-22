use super::{RunStore, entities, run_operations::run_jobs_by_ids};
use crate::error::PostgresError;
use scope_domain::runs::{job::RunJob, run::Run};
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter, QueryOrder, QuerySelect};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RunHistoryCursor {
    pub creation_sequence: u64,
}

pub struct RunHistoryPageQuery<'a> {
    pub repository_id: &'a str,
    pub workflow_path: Option<&'a str>,
    pub after: Option<&'a RunHistoryCursor>,
    pub limit: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RepositoryRun {
    pub run: Run,
    pub jobs: Vec<RunJob>,
    pub creation_sequence: u64,
}

impl RunStore {
    pub async fn repository_run_history_page(
        &self,
        query: RunHistoryPageQuery<'_>,
    ) -> Result<Vec<RepositoryRun>, PostgresError> {
        let tx = super::begin_metadata_read_snapshot(self.db.as_ref()).await?;
        let mut select = entities::run::Entity::find()
            .filter(entities::run::Column::RepoId.eq(query.repository_id));
        if let Some(workflow_path) = query.workflow_path {
            select = select.filter(entities::run::Column::WorkflowPath.eq(workflow_path));
        }
        if let Some(after) = query.after {
            let creation_sequence = i64::try_from(after.creation_sequence).map_err(|_| {
                PostgresError::invalid_input("run history cursor sequence exceeds PostgreSQL range")
            })?;
            select = select.filter(entities::run::Column::CreationSequence.lt(creation_sequence));
        }
        let models = select
            .order_by_desc(entities::run::Column::CreationSequence)
            .limit(query.limit)
            .all(&tx)
            .await
            .map_err(PostgresError::internal)?;
        let run_ids = models.iter().map(|run| run.id.clone()).collect::<Vec<_>>();
        let mut jobs = run_jobs_by_ids(&tx, &run_ids).await?;
        let runs = models
            .into_iter()
            .map(|model| {
                let creation_sequence = u64::try_from(model.creation_sequence).map_err(|_| {
                    PostgresError::internal_message("run creation sequence is negative")
                })?;
                let run = model.try_into_domain()?;
                let jobs = jobs
                    .remove(&run.id)
                    .filter(|jobs| !jobs.is_empty())
                    .ok_or_else(|| {
                        PostgresError::internal_message("run is missing its persisted jobs")
                    })?;
                Ok(RepositoryRun {
                    jobs,
                    run,
                    creation_sequence,
                })
            })
            .collect();
        tx.commit().await.map_err(PostgresError::internal)?;
        runs
    }
}
