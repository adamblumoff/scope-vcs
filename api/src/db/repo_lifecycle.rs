#[cfg(any(test, feature = "memory-metadata"))]
use super::cleanup_queue::remove_matching_pending_repo_storage_cleanup;
#[cfg(any(test, feature = "memory-metadata"))]
use super::repo_effects::apply_repo_effects;
use super::{
    MetadataStore, MetadataStoreInner, acquire_metadata_write_lock,
    cleanup_queue::{complete_pending_repo_storage_cleanup, pending_repo_storage_cleanup_exists},
    entities,
    repo_effects::save_repo_effects,
    repository_from_model,
    repository_rows::insert_repository,
    run_api_db_on,
};
use crate::domain::{
    policy::Visibility,
    repo_actions::{create_repo as create_repo_command, delete_repo as delete_repo_command},
    store::{FirstPushToken, GitPushToken, StoredRepository, repo_id},
};
use crate::error::ApiError;
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter, TransactionTrait};
use std::sync::Arc;

impl MetadataStore {
    pub(crate) fn create_repo_with_init_tokens<F>(
        &self,
        owner_user_id: &str,
        name: &str,
        default_visibility: Visibility,
        first_push_token: FirstPushToken,
        git_push_token: GitPushToken,
        cleanup_pending_storage: F,
    ) -> Result<StoredRepository, ApiError>
    where
        F: FnOnce(&str, &str) -> Result<(), ApiError> + Send + 'static,
    {
        let owner_user_id = owner_user_id.to_string();
        let name = name.to_string();
        match self.inner.as_ref() {
            MetadataStoreInner::Postgres { db, runtime } => {
                let db = Arc::clone(db);
                run_api_db_on(runtime, async move {
                    let tx = db.as_ref().begin().await.map_err(ApiError::internal)?;
                    acquire_metadata_write_lock(&tx).await?;
                    let owner = entities::user::Entity::find_by_id(owner_user_id)
                        .one(&tx)
                        .await
                        .map_err(ApiError::internal)?
                        .ok_or_else(|| {
                            ApiError::internal_message("signed-in user was not persisted")
                        })?
                        .try_into_domain()?;
                    let mutation = create_repo_command(
                        &owner,
                        &name,
                        default_visibility,
                        first_push_token,
                        git_push_token,
                    )?;
                    let repo = mutation.result;
                    if entities::repository::Entity::find_by_id(repo.record.id.clone())
                        .one(&tx)
                        .await
                        .map_err(ApiError::internal)?
                        .is_some()
                    {
                        return Err(ApiError::conflict(format!(
                            "repo {} already exists",
                            repo.record.id
                        )));
                    }

                    if pending_repo_storage_cleanup_exists(&tx, &repo.record.id).await? {
                        cleanup_pending_storage(&repo.record.owner_handle, &repo.record.name)?;
                        complete_pending_repo_storage_cleanup(&tx, &repo.record.id).await?;
                    }

                    insert_repository(&tx, &repo).await?;
                    save_repo_effects(&tx, &mutation.effects).await?;
                    tx.commit().await.map_err(ApiError::internal)?;
                    Ok(repo)
                })
            }
            #[cfg(any(test, feature = "memory-metadata"))]
            MetadataStoreInner::Memory(_) => self.update(move |catalog| {
                let owner = catalog.users.get(&owner_user_id).cloned().ok_or_else(|| {
                    ApiError::internal_message("signed-in user was not persisted")
                })?;
                let mutation = create_repo_command(
                    &owner,
                    &name,
                    default_visibility,
                    first_push_token,
                    git_push_token,
                )?;
                let repo = mutation.result;
                if catalog.repositories.contains_key(&repo.record.id) {
                    return Err(ApiError::conflict(format!(
                        "repo {} already exists",
                        repo.record.id
                    )));
                }

                let had_pending_cleanup = remove_matching_pending_repo_storage_cleanup(
                    &mut catalog.pending_repo_storage_deletions,
                    &repo.record.id,
                );
                if had_pending_cleanup {
                    cleanup_pending_storage(&repo.record.owner_handle, &repo.record.name)?;
                }

                catalog
                    .repositories
                    .insert(repo.record.id.clone(), repo.clone());
                apply_repo_effects(catalog, mutation.effects);
                Ok(repo)
            }),
        }
    }

    pub(crate) fn delete_repo(
        &self,
        owner: &str,
        name: &str,
        user_id: &str,
    ) -> Result<String, ApiError> {
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
                            crate::domain::repo_actions::hidden_repo_not_found(&owner, &name)
                        })?;
                    let repo = repository_from_model(&tx, repo).await?;
                    let mutation = delete_repo_command(&repo, &user_id, &owner, &name)?;

                    entities::repository_invite::Entity::delete_many()
                        .filter(entities::repository_invite::Column::RepoId.eq(repo_id.clone()))
                        .exec(&tx)
                        .await
                        .map_err(ApiError::internal)?;
                    entities::repository_member::Entity::delete_many()
                        .filter(entities::repository_member::Column::RepoId.eq(repo_id.clone()))
                        .exec(&tx)
                        .await
                        .map_err(ApiError::internal)?;
                    entities::repository::Entity::delete_by_id(repo_id.clone())
                        .exec(&tx)
                        .await
                        .map_err(ApiError::internal)?;

                    save_repo_effects(&tx, &mutation.effects).await?;
                    tx.commit().await.map_err(ApiError::internal)?;
                    Ok(mutation.result)
                })
            }
            #[cfg(any(test, feature = "memory-metadata"))]
            MetadataStoreInner::Memory(_) => self.update(move |catalog| {
                let repo = catalog.repositories.get(&repo_id).ok_or_else(|| {
                    crate::domain::repo_actions::hidden_repo_not_found(&owner, &name)
                })?;
                let mutation = delete_repo_command(repo, &user_id, &owner, &name)?;

                catalog
                    .repositories
                    .remove(&repo_id)
                    .expect("repo was already checked");
                apply_repo_effects(catalog, mutation.effects);
                Ok(mutation.result)
            }),
        }
    }
}
