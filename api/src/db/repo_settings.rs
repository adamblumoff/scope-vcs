#[cfg(any(test, feature = "memory-metadata"))]
use super::repo_effects::apply_repo_effects;
use super::{
    MetadataStore, MetadataStoreInner, acquire_metadata_write_lock, entities,
    repo_effects::save_repo_mutation, repository_from_model, run_api_db_on,
};
use crate::domain::store::{RepoSettings, StoredRepository, repo_id};
use crate::domain::{policy::Visibility, repo_actions::update_settings};
use crate::error::ApiError;
use sea_orm::{EntityTrait, TransactionTrait};
use std::sync::Arc;

impl MetadataStore {
    pub(crate) fn update_repo_settings(
        &self,
        owner: &str,
        name: &str,
        user_id: &str,
        settings: RepoSettings,
        default_visibility: Visibility,
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
                    let row = entities::repository::Entity::find_by_id(repo_id.clone())
                        .one(&tx)
                        .await
                        .map_err(ApiError::internal)?
                        .ok_or_else(|| {
                            ApiError::not_found(format!("repo {owner}/{name} not found"))
                        })?;
                    let mut repo = repository_from_model(&tx, row).await?;

                    let mutation =
                        update_settings(&mut repo, &user_id, settings, default_visibility)?;
                    save_repo_mutation(&tx, &repo, &mutation.effects).await?;
                    tx.commit().await.map_err(ApiError::internal)?;
                    Ok(repo)
                })
            }
            #[cfg(any(test, feature = "memory-metadata"))]
            MetadataStoreInner::Memory(_) => self.update(move |catalog| {
                let repo = catalog
                    .repositories
                    .get_mut(&repo_id)
                    .ok_or_else(|| ApiError::not_found(format!("repo {owner}/{name} not found")))?;
                let mutation = update_settings(repo, &user_id, settings, default_visibility)?;
                let updated = repo.clone();
                apply_repo_effects(catalog, mutation.effects);
                Ok(updated)
            }),
        }
    }
}
