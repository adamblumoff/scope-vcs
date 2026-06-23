use super::{
    METADATA_LOCK_KEY, MetadataStore, MetadataStoreInner, acquire_metadata_write_lock, decode_json,
    encode_json, entities,
    repo_cleanup::{
        load_pending_repo_storage_deletions, load_pending_source_blob_deletions,
        queue_pending_repo_storage_deletion, queue_pending_source_blob_deletions,
        remove_matching_pending_repo_storage_cleanup, save_pending_repo_storage_deletions,
        save_pending_source_blob_deletions,
    },
    repository_from_model,
    repository_rows::save_repository_row,
    run_api_db_on,
};
use crate::domain::{
    policy::{ScopePath, Visibility},
    repo_actions::{
        apply_repo_file_visibility, apply_staged_update_for_repo, catalog_error,
        ensure_repo_delete_owner, ensure_repo_owner, ensure_repo_setup_access,
        hidden_repo_not_found, reject_staged_update_for_repo, rejected_staged_update_blobs,
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
                    apply_repo_file_visibility(&mut repo, &user_id, &update_paths, visibility)?;

                    entities::repository::Entity::update_many()
                        .filter(entities::repository::Column::Id.eq(repo_id))
                        .col_expr(
                            entities::repository::Column::Policy,
                            Expr::value(encode_json(&repo.policy)?),
                        )
                        .col_expr(
                            entities::repository::Column::Graph,
                            Expr::value(encode_json(&repo.graph)?),
                        )
                        .exec(&tx)
                        .await
                        .map_err(ApiError::internal)?;
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
                apply_repo_file_visibility(repo, &user_id, &update_paths, visibility)?;
                Ok(repo.clone())
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
                    ensure_repo_owner(&repo, &user_id)?;
                    crate::state::promote_pending_import(&mut repo)?;

                    let record = repo.record.clone();
                    save_repository_row(&tx, &repo).await?;
                    tx.commit().await.map_err(ApiError::internal)?;
                    Ok(record)
                })
            }
            #[cfg(test)]
            MetadataStoreInner::Memory(_) => self.update(move |catalog| {
                let repo = catalog
                    .repositories
                    .get(&repo_id)
                    .ok_or_else(|| ApiError::not_found(format!("repo {owner}/{name} not found")))?;
                ensure_repo_owner(repo, &user_id)?;

                let repo = catalog
                    .repositories
                    .get_mut(&repo_id)
                    .expect("repo was already checked");
                crate::state::promote_pending_import(repo)?;
                Ok(repo.record.clone())
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
                    let updated = update_staged_file_visibility_for_repo(
                        &mut repo,
                        &user_id,
                        &update_paths,
                        visibility,
                    )?;

                    save_repository_row(&tx, &repo).await?;
                    tx.commit().await.map_err(ApiError::internal)?;
                    Ok(updated)
                })
            }
            #[cfg(test)]
            MetadataStoreInner::Memory(_) => self.update(move |catalog| {
                let repo = catalog
                    .repositories
                    .get_mut(&repo_id)
                    .ok_or_else(|| ApiError::not_found(format!("repo {owner}/{name} not found")))?;
                update_staged_file_visibility_for_repo(repo, &user_id, &update_paths, visibility)
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
                    let old_snapshot = repo.git_snapshot.clone();
                    let applied = apply_staged_update_for_repo(&mut repo, &user_id)?;
                    let mut pending_source_blob_deletions =
                        load_pending_source_blob_deletions(&tx).await?;
                    queue_pending_source_blob_deletions(
                        &mut pending_source_blob_deletions,
                        old_snapshot,
                    );

                    save_repository_row(&tx, &repo).await?;
                    save_pending_source_blob_deletions(&tx, &pending_source_blob_deletions).await?;
                    tx.commit().await.map_err(ApiError::internal)?;
                    Ok(applied)
                })
            }
            #[cfg(test)]
            MetadataStoreInner::Memory(_) => self.update(move |catalog| {
                let repo = catalog
                    .repositories
                    .get_mut(&repo_id)
                    .ok_or_else(|| ApiError::not_found(format!("repo {owner}/{name} not found")))?;
                let old_snapshot = repo.git_snapshot.clone();
                let applied = apply_staged_update_for_repo(repo, &user_id)?;
                queue_pending_source_blob_deletions(
                    &mut catalog.pending_source_blob_deletions,
                    old_snapshot,
                );
                Ok(applied)
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
                    let rejected = reject_staged_update_for_repo(&mut repo, &user_id)?;
                    let rejected_blobs = rejected_staged_update_blobs(&rejected);
                    let mut pending_source_blob_deletions =
                        load_pending_source_blob_deletions(&tx).await?;
                    queue_pending_source_blob_deletions(
                        &mut pending_source_blob_deletions,
                        rejected_blobs,
                    );

                    save_repository_row(&tx, &repo).await?;
                    save_pending_source_blob_deletions(&tx, &pending_source_blob_deletions).await?;
                    tx.commit().await.map_err(ApiError::internal)?;
                    Ok(rejected)
                })
            }
            #[cfg(test)]
            MetadataStoreInner::Memory(_) => self.update(move |catalog| {
                let repo = catalog
                    .repositories
                    .get_mut(&repo_id)
                    .ok_or_else(|| ApiError::not_found(format!("repo {owner}/{name} not found")))?;
                let rejected = reject_staged_update_for_repo(repo, &user_id)?;
                queue_pending_source_blob_deletions(
                    &mut catalog.pending_source_blob_deletions,
                    rejected_staged_update_blobs(&rejected),
                );
                Ok(rejected)
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
                    ensure_repo_delete_owner(&repo, &user_id, &owner, &name)?;
                    let source_blobs = repo.source_blobs();
                    let cleanup = RepoStorageCleanup {
                        owner_handle: owner,
                        repo_name: name,
                    };

                    entities::membership::Entity::delete_many()
                        .filter(entities::membership::Column::RepoId.eq(repo_id.clone()))
                        .exec(&tx)
                        .await
                        .map_err(ApiError::internal)?;
                    entities::repository::Entity::delete_by_id(repo_id.clone())
                        .exec(&tx)
                        .await
                        .map_err(ApiError::internal)?;

                    let mut pending_repo_storage_deletions =
                        load_pending_repo_storage_deletions(&tx).await?;
                    queue_pending_repo_storage_deletion(
                        &mut pending_repo_storage_deletions,
                        cleanup,
                    );
                    let mut pending_source_blob_deletions =
                        load_pending_source_blob_deletions(&tx).await?;
                    queue_pending_source_blob_deletions(
                        &mut pending_source_blob_deletions,
                        source_blobs,
                    );
                    save_pending_repo_storage_deletions(&tx, &pending_repo_storage_deletions)
                        .await?;
                    save_pending_source_blob_deletions(&tx, &pending_source_blob_deletions).await?;
                    tx.commit().await.map_err(ApiError::internal)?;
                    Ok(repo_id)
                })
            }
            #[cfg(test)]
            MetadataStoreInner::Memory(_) => self.update(move |catalog| {
                let repo = catalog
                    .repositories
                    .get(&repo_id)
                    .ok_or_else(|| hidden_repo_not_found(&owner, &name))?;
                ensure_repo_delete_owner(repo, &user_id, &owner, &name)?;

                let repo = catalog
                    .repositories
                    .remove(&repo_id)
                    .expect("repo was already checked");
                queue_pending_repo_storage_deletion(
                    &mut catalog.pending_repo_storage_deletions,
                    RepoStorageCleanup {
                        owner_handle: owner,
                        repo_name: name,
                    },
                );
                queue_pending_source_blob_deletions(
                    &mut catalog.pending_source_blob_deletions,
                    repo.source_blobs(),
                );
                Ok(repo_id)
            }),
        }
    }
}
