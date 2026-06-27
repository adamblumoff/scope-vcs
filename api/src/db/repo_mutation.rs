use super::{
    MetadataStore, MetadataStoreInner, acquire_metadata_write_lock,
    cleanup_queue::{
        load_pending_source_blob_deletions, queue_pending_source_blob_deletions,
        save_pending_source_blob_deletions,
    },
    entities, repository_from_model,
    repository_rows::save_repository_row,
    run_api_db_on,
};
use crate::{
    domain::store::{SourceBlob, StoredRepository, repo_id},
    error::ApiError,
};
use sea_orm::{EntityTrait, TransactionTrait};
use std::sync::Arc;

pub(crate) struct RepositoryMutation<R> {
    pub(crate) result: R,
    pub(crate) source_blobs_to_delete: Vec<SourceBlob>,
}

impl<R> RepositoryMutation<R> {
    pub(crate) fn new(result: R) -> Self {
        Self {
            result,
            source_blobs_to_delete: Vec::new(),
        }
    }

    pub(crate) fn with_source_blob_deletions(
        result: R,
        source_blobs_to_delete: Vec<SourceBlob>,
    ) -> Self {
        Self {
            result,
            source_blobs_to_delete,
        }
    }
}

impl MetadataStore {
    pub(crate) fn mutate_repository<R, F>(
        &self,
        owner: &str,
        name: &str,
        op: F,
    ) -> Result<R, ApiError>
    where
        R: Send + 'static,
        F: FnOnce(&mut StoredRepository) -> Result<RepositoryMutation<R>, ApiError>
            + Send
            + 'static,
    {
        let repo_id = repo_id(owner, name);
        let owner = owner.to_string();
        let name = name.to_string();
        match self.inner.as_ref() {
            MetadataStoreInner::Postgres { db, runtime } => {
                let db = Arc::clone(db);
                run_api_db_on(runtime, async move {
                    let tx = db.as_ref().begin().await.map_err(ApiError::internal)?;
                    acquire_metadata_write_lock(&tx).await?;
                    let repo = entities::repository::Entity::find_by_id(repo_id)
                        .one(&tx)
                        .await
                        .map_err(ApiError::internal)?
                        .ok_or_else(|| {
                            ApiError::not_found(format!("repo {owner}/{name} not found"))
                        })?;
                    let mut repo = repository_from_model(&tx, repo).await?;
                    let mutation = op(&mut repo)?;
                    save_repository_row(&tx, &repo).await?;
                    if !mutation.source_blobs_to_delete.is_empty() {
                        let mut pending = load_pending_source_blob_deletions(&tx).await?;
                        queue_pending_source_blob_deletions(
                            &mut pending,
                            mutation.source_blobs_to_delete,
                        );
                        save_pending_source_blob_deletions(&tx, &pending).await?;
                    }
                    tx.commit().await.map_err(ApiError::internal)?;
                    Ok(mutation.result)
                })
            }
            #[cfg(test)]
            MetadataStoreInner::Memory(_) => self.update(move |catalog| {
                let repo = catalog
                    .repositories
                    .get_mut(&repo_id)
                    .ok_or_else(|| ApiError::not_found(format!("repo {owner}/{name} not found")))?;
                let mutation = op(repo)?;
                queue_pending_source_blob_deletions(
                    &mut catalog.pending_source_blob_deletions,
                    mutation.source_blobs_to_delete,
                );
                Ok(mutation.result)
            }),
        }
    }
}
