#[cfg(test)]
use super::repo_effects::apply_repo_effects;
use super::{
    METADATA_LOCK_KEY, MetadataStore, MetadataStoreInner, acquire_metadata_write_lock, decode_json,
    encode_json, entities,
    repo_cleanup::remove_matching_pending_repo_storage_cleanup,
    repo_effects::{save_repo_effects, save_repo_mutation},
    repository_from_model, run_api_db_on,
};
use crate::domain::{
    policy::{ScopePath, Visibility},
    repo_actions::{
        apply_repo_file_visibility, apply_staged_update_for_repo, catalog_error,
        delete_repo_for_repo, ensure_repo_setup_access, hidden_repo_not_found,
        publish_pending_import_for_repo, reject_staged_update_for_repo,
        secretless_first_push_token, update_staged_file_visibility_for_repo,
    },
    store::{
        FirstPushToken, GitPushToken, RepoRecord, RepoStorageCleanup, StagedRepoUpdate,
        StoredRepository, repo_id,
    },
};
use crate::error::ApiError;
use sea_orm::{
    ActiveModelTrait, ColumnTrait, EntityTrait, IntoActiveModel, QueryFilter, TransactionTrait,
    sea_query::Expr,
};
use std::sync::Arc;

impl MetadataStore {
    pub(crate) fn create_repo_with_setup_tokens<F>(
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
                    let mut repo = StoredRepository::new(&owner, &name, default_visibility)
                        .map_err(catalog_error)?;
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

                    let metadata_lock =
                        entities::metadata_lock::Entity::find_by_id(METADATA_LOCK_KEY.to_string())
                            .one(&tx)
                            .await
                            .map_err(ApiError::internal)?
                            .ok_or_else(|| {
                                ApiError::internal_message("metadata lock row is missing")
                            })?;
                    let mut pending_repo_storage_deletions = decode_json::<Vec<RepoStorageCleanup>>(
                        metadata_lock.pending_repo_storage_deletions,
                    )?;
                    let had_pending_cleanup = remove_matching_pending_repo_storage_cleanup(
                        &mut pending_repo_storage_deletions,
                        &repo.record.id,
                    );
                    if had_pending_cleanup {
                        cleanup_pending_storage(&repo.record.owner_handle, &repo.record.name)?;
                        entities::metadata_lock::Entity::update_many()
                            .filter(entities::metadata_lock::Column::Key.eq(METADATA_LOCK_KEY))
                            .col_expr(
                                entities::metadata_lock::Column::PendingRepoStorageDeletions,
                                Expr::value(encode_json(&pending_repo_storage_deletions)?),
                            )
                            .exec(&tx)
                            .await
                            .map_err(ApiError::internal)?;
                    }

                    repo.first_push_token = Some(secretless_first_push_token(first_push_token));
                    repo.git_push_token = Some(git_push_token);
                    entities::repository::Model::from_domain(&repo)?
                        .into_active_model()
                        .insert(&tx)
                        .await
                        .map_err(ApiError::internal)?;
                    for membership in &repo.memberships {
                        entities::membership::Model::from_domain(membership)
                            .into_active_model()
                            .insert(&tx)
                            .await
                            .map_err(ApiError::internal)?;
                    }
                    tx.commit().await.map_err(ApiError::internal)?;
                    Ok(repo)
                })
            }
            #[cfg(test)]
            MetadataStoreInner::Memory(_) => self.update(move |catalog| {
                let owner = catalog.users.get(&owner_user_id).cloned().ok_or_else(|| {
                    ApiError::internal_message("signed-in user was not persisted")
                })?;
                let mut repo = StoredRepository::new(&owner, &name, default_visibility)
                    .map_err(catalog_error)?;
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

                repo.first_push_token = Some(secretless_first_push_token(first_push_token));
                repo.git_push_token = Some(git_push_token);
                catalog
                    .repositories
                    .insert(repo.record.id.clone(), repo.clone());
                Ok(repo)
            }),
        }
    }

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
                    let mutation =
                        apply_repo_file_visibility(&mut repo, &user_id, &update_paths, visibility)?;
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
                let mutation =
                    apply_repo_file_visibility(repo, &user_id, &update_paths, visibility)?;
                let updated = repo.clone();
                apply_repo_effects(catalog, mutation.effects);
                Ok(updated)
            }),
        }
    }

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
                    ensure_repo_setup_access(&repo, &user_id)?;

                    entities::repository::Entity::update_many()
                        .filter(entities::repository::Column::Id.eq(repo_id))
                        .col_expr(
                            entities::repository::Column::FirstPushToken,
                            Expr::value(entities::encode_first_push_token(&first_push_token)?),
                        )
                        .col_expr(
                            entities::repository::Column::GitPushToken,
                            Expr::value(encode_json(&git_push_token)?),
                        )
                        .exec(&tx)
                        .await
                        .map_err(ApiError::internal)?;
                    tx.commit().await.map_err(ApiError::internal)?;

                    repo.first_push_token = Some(secretless_first_push_token(first_push_token));
                    repo.git_push_token = Some(git_push_token);
                    Ok(repo)
                })
            }
            #[cfg(test)]
            MetadataStoreInner::Memory(_) => self.update(move |catalog| {
                let repo = catalog
                    .repositories
                    .get_mut(&repo_id)
                    .ok_or_else(|| ApiError::not_found(format!("repo {owner}/{name} not found")))?;
                ensure_repo_setup_access(repo, &user_id)?;
                repo.first_push_token = Some(secretless_first_push_token(first_push_token));
                repo.git_push_token = Some(git_push_token);
                Ok(repo.clone())
            }),
        }
    }

    pub(crate) fn publish_pending_import(
        &self,
        owner: &str,
        name: &str,
        user_id: &str,
    ) -> Result<RepoRecord, ApiError> {
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
                    let mutation = publish_pending_import_for_repo(&mut repo, &user_id)?;

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
                let mutation = publish_pending_import_for_repo(repo, &user_id)?;
                apply_repo_effects(catalog, mutation.effects);
                Ok(mutation.result)
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
                    let mutation = update_staged_file_visibility_for_repo(
                        &mut repo,
                        &user_id,
                        &update_paths,
                        visibility,
                    )?;
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
                let mutation = update_staged_file_visibility_for_repo(
                    repo,
                    &user_id,
                    &update_paths,
                    visibility,
                )?;
                apply_repo_effects(catalog, mutation.effects);
                Ok(mutation.result)
            }),
        }
    }

    pub(crate) fn apply_staged_update(
        &self,
        owner: &str,
        name: &str,
        user_id: &str,
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
                    let mutation = apply_staged_update_for_repo(&mut repo, &user_id)?;
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
                let mutation = apply_staged_update_for_repo(repo, &user_id)?;
                apply_repo_effects(catalog, mutation.effects);
                Ok(mutation.result)
            }),
        }
    }

    pub(crate) fn reject_staged_update(
        &self,
        owner: &str,
        name: &str,
        user_id: &str,
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
                    let mutation = reject_staged_update_for_repo(&mut repo, &user_id)?;
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
                let mutation = reject_staged_update_for_repo(repo, &user_id)?;
                apply_repo_effects(catalog, mutation.effects);
                Ok(mutation.result)
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
                        .ok_or_else(|| hidden_repo_not_found(&owner, &name))?;
                    let repo = repository_from_model(&tx, repo).await?;
                    let mutation = delete_repo_for_repo(&repo, &user_id, &owner, &name)?;

                    entities::membership::Entity::delete_many()
                        .filter(entities::membership::Column::RepoId.eq(repo_id.clone()))
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
            #[cfg(test)]
            MetadataStoreInner::Memory(_) => self.update(move |catalog| {
                let repo = catalog
                    .repositories
                    .get(&repo_id)
                    .ok_or_else(|| hidden_repo_not_found(&owner, &name))?;
                let mutation = delete_repo_for_repo(repo, &user_id, &owner, &name)?;

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
