use super::{RunStore, entities, run_operations::run_jobs_by_ids};
use crate::error::PostgresError;
use scope_domain::runs::{job::RunJob, run::Run};
use sea_orm::{ColumnTrait, Condition, EntityTrait, QueryFilter, QueryOrder, QuerySelect};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RunHistoryCursor {
    pub created_at_unix: u64,
    pub run_id: String,
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
            let created_at_unix = i64::try_from(after.created_at_unix).map_err(|_| {
                PostgresError::invalid_input("run history cursor time exceeds PostgreSQL range")
            })?;
            select = select.filter(
                Condition::any()
                    .add(entities::run::Column::CreatedAtUnix.lt(created_at_unix))
                    .add(
                        Condition::all()
                            .add(entities::run::Column::CreatedAtUnix.eq(created_at_unix))
                            .add(entities::run::Column::Id.lt(after.run_id.as_str())),
                    ),
            );
        }
        let models = select
            .order_by_desc(entities::run::Column::CreatedAtUnix)
            .order_by_desc(entities::run::Column::Id)
            .limit(query.limit)
            .all(&tx)
            .await
            .map_err(PostgresError::internal)?;
        let run_ids = models.iter().map(|run| run.id.clone()).collect::<Vec<_>>();
        let mut jobs = run_jobs_by_ids(&tx, &run_ids).await?;
        let runs = models
            .into_iter()
            .map(entities::run::Model::try_into_domain)
            .map(|run| {
                let run = run?;
                let jobs = jobs
                    .remove(&run.id)
                    .filter(|jobs| !jobs.is_empty())
                    .ok_or_else(|| {
                        PostgresError::internal_message("run is missing its persisted jobs")
                    })?;
                Ok(RepositoryRun { jobs, run })
            })
            .collect();
        tx.commit().await.map_err(PostgresError::internal)?;
        runs
    }
}
