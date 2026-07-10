use super::{
    MetadataStore, acquire_metadata_write_lock,
    cleanup_queue::queue_pending_source_blob_deletion_rows, entities, repository_from_model,
    repository_rows::save_repository_row,
};
use crate::{
    domain::store::{SourceBlob, StoredRepository, repo_id},
    error::ApiError,
};
use sea_orm::{EntityTrait, TransactionTrait};
use std::sync::Arc;

pub struct RepositoryMutation<R> {
    pub result: R,
    pub source_blobs_to_delete: Vec<SourceBlob>,
}

impl<R> RepositoryMutation<R> {
    pub fn new(result: R) -> Self {
        Self {
            result,
            source_blobs_to_delete: Vec::new(),
        }
    }

    pub fn with_source_blob_deletions(result: R, source_blobs_to_delete: Vec<SourceBlob>) -> Self {
        Self {
            result,
            source_blobs_to_delete,
        }
    }
}

impl MetadataStore {
    pub async fn mutate_repository<R, F>(
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
        let db = Arc::clone(&self.db);
        let tx = db.as_ref().begin().await.map_err(ApiError::internal)?;
        acquire_metadata_write_lock(&tx).await?;
        let repo = entities::repository::Entity::find_by_id(repo_id)
            .one(&tx)
            .await
            .map_err(ApiError::internal)?
            .ok_or_else(|| ApiError::not_found(format!("repo {owner}/{name} not found")))?;
        let mut repo = repository_from_model(&tx, repo).await?;
        let mutation = op(&mut repo)?;
        save_repository_row(&tx, &repo).await?;
        if !mutation.source_blobs_to_delete.is_empty() {
            queue_pending_source_blob_deletion_rows(&tx, mutation.source_blobs_to_delete).await?;
        }
        tx.commit().await.map_err(ApiError::internal)?;
        Ok(mutation.result)
    }
}
