use super::{
    mapping::encode_content_ref,
    types::{SourceBlobCleanupBatch, SourceBlobCleanupDecision},
};
use crate::{
    db::{CleanupStore, entities},
    error::PostgresError,
};
use scope_domain::{content::SourceBlob, repo_actions::RepoStorageCleanup, repository::repo_id};
use sea_orm::{ColumnTrait, ConnectionTrait, EntityTrait, QueryFilter, TransactionTrait};
use std::collections::BTreeSet;

impl CleanupStore {
    /// Revalidates one claimed object immediately before physical deletion.
    /// Callers must hold that object's content-ref fence through this check and deletion.
    pub async fn source_blob_cleanup_decision(
        &self,
        batch: &SourceBlobCleanupBatch,
        blob: &SourceBlob,
    ) -> Result<SourceBlobCleanupDecision, PostgresError> {
        let Some(loaded) = batch
            .loaded
            .iter()
            .find(|loaded| loaded.blob.content_ref == blob.content_ref)
        else {
            return Ok(SourceBlobCleanupDecision::StaleClaim);
        };
        let encoded = encode_content_ref(&blob.content_ref)?;
        let tx = self.db.begin().await.map_err(PostgresError::internal)?;
        let job = entities::source_blob_cleanup_job::Entity::find_by_id(encoded.clone())
            .one(&tx)
            .await
            .map_err(PostgresError::internal)?;
        let claim_is_live = job.is_some_and(|job| {
            job.generation == loaded.generation && job.completed_at_unix.is_none()
        });
        if !claim_is_live {
            tx.commit().await.map_err(PostgresError::internal)?;
            return Ok(SourceBlobCleanupDecision::StaleClaim);
        }
        let is_referenced = entities::object_reference::Entity::find()
            .filter(entities::object_reference::Column::ObjectKey.eq(encoded))
            .one(&tx)
            .await
            .map_err(PostgresError::internal)?
            .is_some();
        tx.commit().await.map_err(PostgresError::internal)?;
        Ok(if is_referenced {
            SourceBlobCleanupDecision::Referenced
        } else {
            SourceBlobCleanupDecision::Delete
        })
    }
}

pub(super) async fn live_repo_ids_for_cleanups<C>(
    conn: &C,
    pending: &[RepoStorageCleanup],
) -> Result<BTreeSet<String>, PostgresError>
where
    C: ConnectionTrait,
{
    let cleanup_repo_ids = pending
        .iter()
        .map(|cleanup| repo_id(&cleanup.owner_handle, &cleanup.repo_name))
        .collect::<Vec<_>>();
    if cleanup_repo_ids.is_empty() {
        return Ok(BTreeSet::new());
    }

    let repositories = entities::repository::Entity::find()
        .filter(entities::repository::Column::Id.is_in(cleanup_repo_ids))
        .all(conn)
        .await
        .map_err(PostgresError::internal)?;
    Ok(repositories.into_iter().map(|repo| repo.id).collect())
}
