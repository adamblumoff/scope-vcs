use super::{
    METADATA_LOCK_KEY, MetadataStore, MetadataStoreInner, acquire_metadata_write_lock, decode_json,
    encode_json, entities, repository_from_model, run_api_db_on,
};
use crate::domain::{
    policy::{ScopePath, Visibility, VisibilityRule},
    projection::{AuthorVisibility, FileVisibilityChange, LogicalCommit, MixedCommitPolicy},
    store::{
        CatalogError, FirstPushToken, GitPushToken, RepoPublicationState, RepoRecord, RepoRole,
        RepoSettings, RepoStorageCleanup, SourceBlob, StagedRepoUpdate, StoredRepository, repo_id,
    },
};
use crate::error::ApiError;
use sea_orm::{
    ActiveModelTrait, ColumnTrait, ConnectionTrait, EntityTrait, IntoActiveModel, QueryFilter,
    TransactionTrait, sea_query::Expr,
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

    pub(crate) fn update_repo_settings(
        &self,
        owner: &str,
        name: &str,
        user_id: &str,
        settings: RepoSettings,
    ) -> Result<RepoSettings, ApiError> {
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
                    if entities::repository::Entity::find_by_id(repo_id.clone())
                        .one(&tx)
                        .await
                        .map_err(ApiError::internal)?
                        .is_none()
                    {
                        return Err(ApiError::not_found(format!(
                            "repo {owner}/{name} not found"
                        )));
                    }

                    let role = entities::membership::Entity::find()
                        .filter(entities::membership::Column::RepoId.eq(repo_id.clone()))
                        .filter(entities::membership::Column::UserId.eq(user_id))
                        .one(&tx)
                        .await
                        .map_err(ApiError::internal)?
                        .map(|membership| membership.try_into_domain())
                        .transpose()?
                        .map(|membership| membership.role);
                    if role != Some(RepoRole::Owner) {
                        return Err(ApiError::forbidden("owner role required"));
                    }

                    entities::repository::Entity::update_many()
                        .filter(entities::repository::Column::Id.eq(repo_id))
                        .col_expr(
                            entities::repository::Column::Settings,
                            Expr::value(encode_json(&settings)?),
                        )
                        .exec(&tx)
                        .await
                        .map_err(ApiError::internal)?;
                    tx.commit().await.map_err(ApiError::internal)?;
                    Ok(settings)
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
                repo.settings = settings;
                Ok(settings)
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
}

fn ensure_repo_owner(repo: &StoredRepository, user_id: &str) -> Result<(), ApiError> {
    let role = repo
        .memberships
        .iter()
        .find(|membership| membership.user_id == user_id)
        .map(|membership| membership.role);
    if role != Some(RepoRole::Owner) {
        return Err(ApiError::forbidden("owner role required"));
    }
    Ok(())
}

fn update_staged_file_visibility_for_repo(
    repo: &mut StoredRepository,
    user_id: &str,
    update_paths: &[ScopePath],
    visibility: Visibility,
) -> Result<StagedRepoUpdate, ApiError> {
    if update_paths.is_empty() {
        return Err(ApiError::bad_request("at least one file path is required"));
    }
    ensure_repo_owner(repo, user_id)?;

    let mut staged_update = repo
        .staged_update
        .clone()
        .ok_or_else(|| ApiError::not_found("no staged update pending"))?;
    for path in update_paths {
        let file = staged_update
            .changes
            .iter_mut()
            .find(|change| change.path == *path)
            .ok_or_else(|| ApiError::not_found(format!("staged file {} not found", path)))?;
        file.visibility = visibility;
    }
    crate::git::import::validate_staged_update_policy(repo, &staged_update)?;
    repo.staged_update = Some(staged_update.clone());
    Ok(staged_update)
}

fn apply_staged_update_for_repo(
    repo: &mut StoredRepository,
    user_id: &str,
) -> Result<StagedRepoUpdate, ApiError> {
    ensure_repo_owner(repo, user_id)?;
    let staged_update = repo
        .staged_update
        .take()
        .ok_or_else(|| ApiError::not_found("no staged update pending"))?;
    let applied = staged_update.clone();
    crate::git::import::apply_receive_pack_update(repo, staged_update)?;
    Ok(applied)
}

fn reject_staged_update_for_repo(
    repo: &mut StoredRepository,
    user_id: &str,
) -> Result<StagedRepoUpdate, ApiError> {
    ensure_repo_owner(repo, user_id)?;
    repo.staged_update
        .take()
        .ok_or_else(|| ApiError::not_found("no staged update pending"))
}

fn rejected_staged_update_blobs(staged_update: &StagedRepoUpdate) -> Vec<SourceBlob> {
    std::iter::once(staged_update.git_snapshot.clone())
        .chain(
            staged_update
                .changes
                .iter()
                .filter_map(|change| change.new_content.clone()),
        )
        .collect()
}

fn queue_pending_source_blob_deletions(
    pending: &mut Vec<SourceBlob>,
    blobs: impl IntoIterator<Item = SourceBlob>,
) {
    let mut queued = pending
        .iter()
        .map(|blob| blob.object_key.clone())
        .collect::<std::collections::BTreeSet<_>>();
    for blob in blobs {
        if queued.insert(blob.object_key.clone()) {
            pending.push(blob);
        }
    }
}

async fn load_pending_source_blob_deletions<C>(conn: &C) -> Result<Vec<SourceBlob>, ApiError>
where
    C: ConnectionTrait,
{
    let metadata_lock = entities::metadata_lock::Entity::find_by_id(METADATA_LOCK_KEY.to_string())
        .one(conn)
        .await
        .map_err(ApiError::internal)?
        .ok_or_else(|| ApiError::internal_message("metadata lock row is missing"))?;
    decode_json::<Vec<SourceBlob>>(metadata_lock.pending_source_blob_deletions)
}

async fn save_pending_source_blob_deletions<C>(
    conn: &C,
    pending_source_blob_deletions: &[SourceBlob],
) -> Result<(), ApiError>
where
    C: ConnectionTrait,
{
    entities::metadata_lock::Entity::update_many()
        .filter(entities::metadata_lock::Column::Key.eq(METADATA_LOCK_KEY))
        .col_expr(
            entities::metadata_lock::Column::PendingSourceBlobDeletions,
            Expr::value(encode_json(&pending_source_blob_deletions)?),
        )
        .exec(conn)
        .await
        .map_err(ApiError::internal)?;
    Ok(())
}

async fn save_repository_row<C>(conn: &C, repo: &StoredRepository) -> Result<(), ApiError>
where
    C: ConnectionTrait,
{
    entities::repository::Model::from_domain(repo)?
        .into_active_model()
        .update(conn)
        .await
        .map_err(ApiError::internal)?;
    Ok(())
}

fn ensure_repo_setup_access(repo: &StoredRepository, user_id: &str) -> Result<(), ApiError> {
    let role = repo
        .memberships
        .iter()
        .find(|membership| membership.user_id == user_id)
        .map(|membership| membership.role);
    if role != Some(RepoRole::Owner) {
        return Err(ApiError::not_found(format!(
            "repo {} not found",
            repo.record.id
        )));
    }
    if repo.record.publication_state != RepoPublicationState::PendingFirstPush {
        return Err(ApiError::conflict(
            "setup token is only available before the first push",
        ));
    }
    Ok(())
}

fn secretless_first_push_token(mut token: FirstPushToken) -> FirstPushToken {
    token.secret = None;
    token
}

fn remove_matching_pending_repo_storage_cleanup(
    cleanups: &mut Vec<RepoStorageCleanup>,
    cleanup_repo_id: &str,
) -> bool {
    let original_len = cleanups.len();
    cleanups
        .retain(|cleanup| repo_id(&cleanup.owner_handle, &cleanup.repo_name) != cleanup_repo_id);
    cleanups.len() != original_len
}

fn catalog_error(error: CatalogError) -> ApiError {
    match error {
        CatalogError::InvalidRepositoryName(message) => ApiError::bad_request(message),
        CatalogError::RepositoryExists(repo) => {
            ApiError::conflict(format!("repo {repo} already exists"))
        }
    }
}

fn apply_repo_file_visibility(
    repo: &mut StoredRepository,
    user_id: &str,
    update_paths: &[ScopePath],
    visibility: Visibility,
) -> Result<(), ApiError> {
    if update_paths.is_empty() {
        return Err(ApiError::bad_request("at least one file path is required"));
    }
    ensure_repo_owner(repo, user_id)?;
    if visibility == Visibility::Public {
        for update_path in update_paths {
            if !repo.has_file_for_visibility_update(update_path) {
                return Err(ApiError::bad_request(format!(
                    "file {} must be tracked by Git before it can be made public",
                    update_path.as_str()
                )));
            }
        }
    }

    let owner_ids = repo.owner_ids();
    let record_visibility_history =
        repo.record.publication_state == RepoPublicationState::Published;
    let live_tree = if record_visibility_history {
        repo.live_tree()
    } else {
        Default::default()
    };
    let mut visibility_changes = Vec::new();
    for update_path in update_paths {
        let old_visibility = repo.policy.effective_visibility(update_path);
        if record_visibility_history && old_visibility != visibility {
            visibility_changes.push(FileVisibilityChange {
                path: update_path.clone(),
                old_visibility,
                new_visibility: visibility,
                current_content: live_tree.get(update_path).cloned(),
            });
        }
        let rule = match visibility {
            Visibility::Public => VisibilityRule::public(update_path.clone()),
            Visibility::Private => VisibilityRule::private(update_path.clone(), owner_ids.clone()),
        };
        repo.policy.add_rule(rule).map_err(ApiError::bad_request)?;
    }
    if !visibility_changes.is_empty() {
        let parent_ids = repo
            .graph
            .commits
            .last()
            .map(|commit| vec![commit.id.clone()])
            .unwrap_or_default();
        repo.graph.commits.push(LogicalCommit {
            id: format!("rv_visibility_{}", repo.graph.commits.len() + 1),
            parent_ids,
            author_id: user_id.to_string(),
            author_visibility: AuthorVisibility::Visible,
            message: "Update file visibility".to_string(),
            mixed_policy: MixedCommitPolicy::SyntheticPublicCommit,
            changes: Vec::new(),
            visibility_changes,
        });
    }
    Ok(())
}
