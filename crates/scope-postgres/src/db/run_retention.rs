use super::{
    GeneratedIdSource, RunStore, cleanup_queue::queue_pending_source_blob_deletion_rows, entities,
};
use crate::error::PostgresError;
use scope_domain::store::SourceBlob;
use sea_orm::{
    ColumnTrait, EntityTrait, QueryFilter, QueryOrder, QuerySelect, TransactionTrait,
    sea_query::Query,
};
use std::collections::BTreeSet;

impl RunStore {
    pub async fn prune_terminal_runs(
        &self,
        completed_before_unix: u64,
        now_unix: u64,
        limit: u64,
        generated_ids: &dyn GeneratedIdSource,
    ) -> Result<usize, PostgresError> {
        let cutoff = entities::u64_to_i64(completed_before_unix, "run retention cutoff")?;
        let tx = self.db.begin().await.map_err(PostgresError::internal)?;
        let models = entities::run::Entity::find()
            .filter(entities::run::Column::State.is_in([
                "succeeded".to_string(),
                "failed".to_string(),
                "canceled".to_string(),
                "lost".to_string(),
            ]))
            .filter(entities::run::Column::CompletedAtUnix.lte(cutoff))
            .order_by_asc(entities::run::Column::CompletedAtUnix)
            .order_by_asc(entities::run::Column::Id)
            .limit(limit)
            .lock_exclusive()
            .all(&tx)
            .await
            .map_err(PostgresError::internal)?;
        let runs = models
            .into_iter()
            .map(entities::run::Model::try_into_domain)
            .collect::<Result<Vec<_>, _>>()?;
        let run_ids = runs.iter().map(|run| run.id.clone()).collect::<Vec<_>>();
        if run_ids.is_empty() {
            tx.commit().await.map_err(PostgresError::internal)?;
            return Ok(0);
        }
        let sources = runs
            .iter()
            .flat_map(|run| run.source.retained_objects())
            .cloned()
            .collect::<Vec<SourceBlob>>();
        let workflow_digests = runs
            .iter()
            .map(|run| run.workflow_revision_digest.clone())
            .collect::<BTreeSet<_>>();

        entities::run_log::Entity::delete_many()
            .filter(entities::run_log::Column::RunId.is_in(run_ids.clone()))
            .exec(&tx)
            .await
            .map_err(PostgresError::internal)?;
        let attempt_ids = entities::run_attempt::Entity::find()
            .select_only()
            .column(entities::run_attempt::Column::Id)
            .filter(entities::run_attempt::Column::RunId.is_in(run_ids.clone()))
            .into_tuple::<String>()
            .all(&tx)
            .await
            .map_err(PostgresError::internal)?;
        entities::run_attempt_step::Entity::delete_many()
            .filter(entities::run_attempt_step::Column::AttemptId.is_in(attempt_ids))
            .exec(&tx)
            .await
            .map_err(PostgresError::internal)?;
        entities::run_attempt::Entity::delete_many()
            .filter(entities::run_attempt::Column::RunId.is_in(run_ids.clone()))
            .exec(&tx)
            .await
            .map_err(PostgresError::internal)?;
        entities::object_reference::Entity::delete_many()
            .filter(entities::object_reference::Column::RefKind.eq("run_source"))
            .filter(entities::object_reference::Column::RefId.is_in(run_ids.clone()))
            .exec(&tx)
            .await
            .map_err(PostgresError::internal)?;
        entities::run::Entity::delete_many()
            .filter(entities::run::Column::Id.is_in(run_ids))
            .exec(&tx)
            .await
            .map_err(PostgresError::internal)?;
        entities::workflow_revision::Entity::delete_many()
            .filter(entities::workflow_revision::Column::Digest.is_in(workflow_digests))
            .filter(
                entities::workflow_revision::Column::Digest.not_in_subquery(
                    Query::select()
                        .column(entities::run::Column::WorkflowRevisionDigest)
                        .from(entities::run::Entity)
                        .to_owned(),
                ),
            )
            .exec(&tx)
            .await
            .map_err(PostgresError::internal)?;
        queue_pending_source_blob_deletion_rows(&tx, sources.clone(), now_unix, generated_ids)
            .await?;
        tx.commit().await.map_err(PostgresError::internal)?;

        Ok(sources.len())
    }
}
