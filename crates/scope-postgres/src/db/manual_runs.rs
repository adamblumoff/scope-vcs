use super::{
    EnqueueRunResult, RunStore, acquire_aggregate_lock, entities,
    git_push_reads::git_push_context_for_id, runs::enqueue_run_in_transaction,
    workflow_catalogs::repository_workflow_catalog,
};
use crate::error::PostgresError;
use scope_domain::{
    projection::ProjectionViewKey,
    runs::{manual::ManualRunRequest, source::RunSource},
};
use sea_orm::{EntityTrait, TransactionTrait};

impl RunStore {
    /// Pins a known source and its workflow while repository mutation is excluded.
    /// A missing exact source leaves no run behind, allowing the caller to upload it.
    pub async fn enqueue_known_manual_run(
        &self,
        request: &ManualRunRequest,
        now_unix: u64,
    ) -> Result<Option<EnqueueRunResult>, PostgresError> {
        let tx = self.db.begin().await.map_err(PostgresError::internal)?;
        acquire_aggregate_lock(&tx, "repository", request.repository_id()).await?;
        let context = git_push_context_for_id(&tx, request.repository_id(), request.user_id())
            .await?
            .ok_or_else(|| PostgresError::not_found("repo not found"))?;
        request.require_access(context.access)?;
        if let Some(stored) = entities::run::Entity::find_by_id(request.run_id())
            .one(&tx)
            .await
            .map_err(PostgresError::internal)?
        {
            let run = stored.try_into_domain()?;
            request.require_matching_run(&run)?;
            tx.commit().await.map_err(PostgresError::internal)?;
            return Ok(Some(EnqueueRunResult {
                run,
                inserted: false,
            }));
        }
        let Some(head) = context
            .git_head
            .filter(|head| head.head_oid == request.git_oid())
        else {
            return Ok(None);
        };
        let Some(catalog) = repository_workflow_catalog(&tx, request.repository_id()).await? else {
            return Ok(None);
        };
        if catalog.source_head_oid() != head.head_oid
            || catalog.source_change_version() != head.change_version
        {
            return Ok(None);
        }
        catalog
            .verify_source(request.repository_id(), &head.head_oid, head.change_version)
            .map_err(PostgresError::internal)?;
        let workflow = request.workflow_file(&catalog)?;
        let revision =
            scope_run_config::parse_workflow(workflow.path().as_str(), workflow.content_bytes())
                .map_err(PostgresError::invalid_input)?
                .into_revision(request.repository_id())
                .map_err(PostgresError::invalid_input)?;
        let source = RunSource::accepted_git_head(
            request.repository_id(),
            head,
            context.git_pack_spans,
            ProjectionViewKey::Private,
        )?;
        let run = request.create_run(&revision, source, now_unix)?;
        let enqueued = enqueue_run_in_transaction(&tx, run, revision).await?;
        tx.commit().await.map_err(PostgresError::internal)?;
        Ok(Some(enqueued))
    }
}
