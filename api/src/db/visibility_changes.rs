#[cfg(test)]
use super::repo_effects::apply_repo_effects;
use super::{
    MetadataStore, MetadataStoreInner, acquire_metadata_write_lock, entities,
    repo_effects::save_repo_mutation, repository_from_model, run_api_db_on,
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
    pub(crate) fn update_repo_file_visibility(
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
        match self.inner.as_ref() {
            MetadataStoreInner::Postgres { db, runtime } => {
                let db = Arc::clone(db);
                run_api_db_on(runtime, async move {
                    let tx = db.as_ref().begin().await.map_err(ApiError::internal)?;
                    acquire_metadata_write_lock(&tx).await?;
                    let repo = entities::repository::Entity::find_by_id(repo_id.clone())
                        .one(&tx)
                        .await
                        .map_err(ApiError::internal)?
                        .ok_or_else(|| {
                            ApiError::not_found(format!("repo {owner}/{name} not found"))
                        })?;
                    let mut repo = repository_from_model(&tx, repo).await?;
                    let mutation = set_visibility(&mut repo, &user_id, &update_paths, visibility)?;
                    save_repo_mutation(&tx, &repo, &mutation.effects).await?;
                    tx.commit().await.map_err(ApiError::internal)?;
                    Ok(repo)
                })
            }
            #[cfg(test)]
            MetadataStoreInner::Memory(_) => self.update(move |catalog| {
                let repo = catalog
                    .repositories
                    .get_mut(&repo_id)
                    .ok_or_else(|| ApiError::not_found(format!("repo {owner}/{name} not found")))?;
                let mutation = set_visibility(repo, &user_id, &update_paths, visibility)?;
                let updated = repo.clone();
                apply_repo_effects(catalog, mutation.effects);
                Ok(updated)
            }),
        }
    }
}
