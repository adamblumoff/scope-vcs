use super::{RunStore, entities};
use crate::error::PostgresError;
use scope_domain::runs::{
    run::Run,
    runner::{Runner, RunnerGrant},
};
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter, QueryOrder, QuerySelect};
use std::collections::BTreeMap;

use super::StoredRunLog;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RepositoryRunner {
    pub runner: Runner,
    pub grant: RunnerGrant,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RecentRunLogs {
    pub logs: Vec<StoredRunLog>,
    pub truncated_in_view: bool,
}

impl RunStore {
    pub async fn repository_operations_runs(
        &self,
        repository_id: &str,
        recent_limit: u64,
    ) -> Result<Vec<Run>, PostgresError> {
        let mut models = entities::run::Entity::find()
            .filter(entities::run::Column::RepoId.eq(repository_id))
            .filter(entities::run::Column::CompletedAtUnix.is_null())
            .order_by_desc(entities::run::Column::UpdatedAtUnix)
            .order_by_desc(entities::run::Column::Id)
            .all(self.db.as_ref())
            .await
            .map_err(PostgresError::internal)?;
        let terminal_limit = recent_limit.saturating_sub(models.len() as u64);
        if terminal_limit > 0 {
            let active_run_ids = models.iter().map(|run| run.id.clone()).collect::<Vec<_>>();
            let mut terminal_query = entities::run::Entity::find()
                .filter(entities::run::Column::RepoId.eq(repository_id))
                .filter(entities::run::Column::CompletedAtUnix.is_not_null())
                .order_by_desc(entities::run::Column::UpdatedAtUnix)
                .order_by_desc(entities::run::Column::Id)
                .limit(terminal_limit);
            if !active_run_ids.is_empty() {
                terminal_query =
                    terminal_query.filter(entities::run::Column::Id.is_not_in(active_run_ids));
            }
            models.extend(
                terminal_query
                    .all(self.db.as_ref())
                    .await
                    .map_err(PostgresError::internal)?,
            );
        }
        models
            .into_iter()
            .map(entities::run::Model::try_into_domain)
            .collect()
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

        Ok(RecentRunLogs {
            truncated_in_view,
            logs: logs
                .into_iter()
                .map(|model| {
                    let position = entities::i64_to_u64(model.position, "run log position")?;
                    let run_id = model.run_id.clone();
                    Ok(StoredRunLog {
                        position,
                        run_id,
                        chunk: model.try_into_domain()?,
                    })
                })
                .collect::<Result<_, PostgresError>>()?,
        })
    }

    pub async fn run_has_truncated_logs(&self, run_id: &str) -> Result<bool, PostgresError> {
        Ok(entities::run_attempt::Entity::find()
            .filter(entities::run_attempt::Column::RunId.eq(run_id))
            .filter(entities::run_attempt::Column::LogsTruncated.eq(true))
            .one(self.db.as_ref())
            .await
            .map_err(PostgresError::internal)?
            .is_some())
    }
}
