#[cfg(test)]
use super::repo_effects::apply_repo_effects;
use super::{
    MetadataStore, MetadataStoreInner, acquire_metadata_write_lock, entities,
    repo_effects::save_repo_mutation, repository_from_model, run_api_db_on,
};
use crate::domain::{
    policy::{ScopePath, Visibility},
    repo_actions::{
        regenerate_setup_tokens as regenerate_setup_tokens_command, set_staged_visibility,
    },
    store::{FirstPushToken, GitPushToken, StagedRepoUpdate, StoredRepository, repo_id},
};
use crate::error::ApiError;
use sea_orm::{EntityTrait, TransactionTrait};
use std::sync::Arc;

impl MetadataStore {
    pub(crate) fn regenerate_repo_setup_tokens(
        &self,
        owner: &str,
        name: &str,
        user_id: &str,
        first_push_token: FirstPushToken,
        git_push_token: GitPushToken,
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
                    let mutation = regenerate_setup_tokens_command(
                        &mut repo,
                        &user_id,
                        first_push_token,
                        git_push_token,
                    )?;

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
                let mutation = regenerate_setup_tokens_command(
                    repo,
                    &user_id,
                    first_push_token,
                    git_push_token,
                )?;
                let updated = repo.clone();
                apply_repo_effects(catalog, mutation.effects);
                Ok(updated)
            }),
        }
    }

    pub(crate) fn update_staged_file_visibility(
        &self,
        owner: &str,
        name: &str,
        user_id: &str,
        update_paths: Vec<ScopePath>,
        visibility: Visibility,
    ) -> Result<StagedRepoUpdate, ApiError> {
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
                    let repo = entities::repository::Entity::find_by_id(repo_id)
                        .one(&tx)
                        .await
                        .map_err(ApiError::internal)?
                        .ok_or_else(|| {
                            ApiError::not_found(format!("repo {owner}/{name} not found"))
                        })?;
                    let mut repo = repository_from_model(&tx, repo).await?;
                    let mutation =
                        set_staged_visibility(&mut repo, &user_id, &update_paths, visibility)?;
                    save_repo_mutation(&tx, &repo, &mutation.effects).await?;
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
                let mutation = set_staged_visibility(repo, &user_id, &update_paths, visibility)?;
                apply_repo_effects(catalog, mutation.effects);
                Ok(mutation.result)
            }),
        }
    }
}
