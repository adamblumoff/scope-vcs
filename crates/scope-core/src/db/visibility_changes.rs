use super::{
    MetadataStore, acquire_metadata_write_lock, entities, repo_effects::save_repo_mutation,
    repository_from_model,
};
use crate::domain::{
    policy::{ScopePath, Visibility},
    repo_actions::set_visibility,
    store::{StoredRepository, repo_id},
};
use crate::error::ApiError;
use sea_orm::{EntityTrait, TransactionTrait};
use std::sync::Arc;

impl MetadataStore {
    pub async fn update_repo_file_visibility(
        &self,
        owner: &str,
        name: &str,
        user_id: &str,
        update_paths: Vec<ScopePath>,
        visibility: Visibility,
    ) -> Result<StoredRepository, ApiError> {
        let repo_id = repo_id(owner, name);
        let owner = owner.to_string();
        let name = name.to_string();
        let user_id = user_id.to_string();
        let db = Arc::clone(&self.db);
        let tx = db.as_ref().begin().await.map_err(ApiError::internal)?;
        acquire_metadata_write_lock(&tx).await?;
        let repo = entities::repository::Entity::find_by_id(repo_id.clone())
            .one(&tx)
            .await
            .map_err(ApiError::internal)?
            .ok_or_else(|| ApiError::not_found(format!("repo {owner}/{name} not found")))?;
        let mut repo = repository_from_model(&tx, repo).await?;
        let mutation = set_visibility(&mut repo, &user_id, &update_paths, visibility)?;
        save_repo_mutation(&tx, &repo, &mutation.effects).await?;
        tx.commit().await.map_err(ApiError::internal)?;
        Ok(repo)
    }
}
