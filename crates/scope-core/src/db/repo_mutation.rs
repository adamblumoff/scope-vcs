use super::{
    MetadataStore, acquire_aggregate_lock, cleanup_queue::queue_pending_source_blob_deletion_rows,
    entities, repository_from_model, repository_rows::save_repository_delta,
};
use crate::{
    domain::store::{SourceBlob, StoredRepository, repo_id},
    error::ApiError,
};
use sea_orm::{EntityTrait, TransactionTrait};
use std::sync::Arc;

pub struct RepositoryMutation<R> {
    pub result: R,
    pub orphan_objects: Vec<SourceBlob>,
}

impl<R> RepositoryMutation<R> {
    pub fn new(result: R) -> Self {
        Self {
            result,
            orphan_objects: Vec::new(),
        }
    }

    pub fn with_source_blob_deletions(result: R, orphan_objects: Vec<SourceBlob>) -> Self {
        Self {
            result,
            orphan_objects,
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
        acquire_aggregate_lock(&tx, "repository", &repo_id).await?;
        let repo = entities::repository::Entity::find_by_id(repo_id)
            .one(&tx)
            .await
            .map_err(ApiError::internal)?
            .ok_or_else(|| ApiError::not_found(format!("repo {owner}/{name} not found")))?;
        let mut repo = repository_from_model(&tx, repo).await?;
        let before = repo.clone();
        let mutation = op(&mut repo)?;
        save_repository_delta(&tx, &before, &repo).await?;
        if !mutation.orphan_objects.is_empty() {
            queue_pending_source_blob_deletion_rows(&tx, mutation.orphan_objects).await?;
        }
        tx.commit().await.map_err(ApiError::internal)?;
        Ok(mutation.result)
    }
}
