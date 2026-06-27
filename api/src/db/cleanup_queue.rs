use super::{
    METADATA_LOCK_KEY, MetadataStore, MetadataStoreInner, acquire_metadata_write_lock, decode_json,
    encode_json, entities, repositories_from_models, run_api_db_on,
};
use crate::domain::store::{RepoStorageCleanup, SourceBlob, repo_id};
use crate::error::ApiError;
use sea_orm::{
    ColumnTrait, ConnectionTrait, EntityTrait, QueryFilter, QueryOrder, TransactionTrait,
    sea_query::Expr,
};
use std::{collections::BTreeSet, sync::Arc};

impl MetadataStore {
    pub(crate) fn queue_pending_source_blob_deletions(
        &self,
        blobs: Vec<SourceBlob>,
    ) -> Result<(), ApiError> {
        if blobs.is_empty() {
            return Ok(());
        }

        match self.inner.as_ref() {
            MetadataStoreInner::Postgres { db, runtime } => {
                let db = Arc::clone(db);
                run_api_db_on(runtime, async move {
                    let tx = db.as_ref().begin().await.map_err(ApiError::internal)?;
                    acquire_metadata_write_lock(&tx).await?;
                    let mut pending = load_pending_source_blob_deletions(&tx).await?;
                    queue_pending_source_blob_deletions(&mut pending, blobs);
                    save_pending_source_blob_deletions(&tx, &pending).await?;
                    tx.commit().await.map_err(ApiError::internal)?;
                    Ok(())
                })
            }
            #[cfg(any(test, feature = "memory-metadata"))]
            MetadataStoreInner::Memory(_) => self.update(move |catalog| {
                queue_pending_source_blob_deletions(
                    &mut catalog.pending_source_blob_deletions,
                    blobs,
                );
                Ok(())
            }),
        }
    }

    pub(crate) fn update_pending_repo_storage_deletions<R, F>(&self, op: F) -> Result<R, ApiError>
    where
        R: Send + 'static,
        F: FnOnce(
                Vec<RepoStorageCleanup>,
                &BTreeSet<String>,
            ) -> Result<(R, Vec<RepoStorageCleanup>), ApiError>
            + Send
            + 'static,
    {
        match self.inner.as_ref() {
            MetadataStoreInner::Postgres { db, runtime } => {
                let db = Arc::clone(db);
                run_api_db_on(runtime, async move {
                    let tx = db.as_ref().begin().await.map_err(ApiError::internal)?;
                    acquire_metadata_write_lock(&tx).await?;
                    let pending = load_pending_repo_storage_deletions(&tx).await?;
                    let live_repo_ids = live_repo_ids_for_cleanups(&tx, &pending).await?;
                    let (result, retained) = op(pending, &live_repo_ids)?;
                    save_pending_repo_storage_deletions(&tx, &retained).await?;
                    tx.commit().await.map_err(ApiError::internal)?;
                    Ok(result)
                })
            }
            #[cfg(any(test, feature = "memory-metadata"))]
            MetadataStoreInner::Memory(_) => self.update(move |catalog| {
                let live_repo_ids = catalog
                    .repositories
                    .keys()
                    .cloned()
                    .collect::<BTreeSet<_>>();
                let pending = std::mem::take(&mut catalog.pending_repo_storage_deletions);
                let (result, retained) = op(pending, &live_repo_ids)?;
                catalog.pending_repo_storage_deletions = retained;
                Ok(result)
            }),
        }
    }

    pub(crate) fn update_pending_source_blob_deletions<R, F>(&self, op: F) -> Result<R, ApiError>
    where
        R: Send + 'static,
        F: FnOnce(Vec<SourceBlob>, &BTreeSet<String>) -> Result<(R, Vec<SourceBlob>), ApiError>
            + Send
            + 'static,
    {
        match self.inner.as_ref() {
            MetadataStoreInner::Postgres { db, runtime } => {
                let db = Arc::clone(db);
                run_api_db_on(runtime, async move {
                    let tx = db.as_ref().begin().await.map_err(ApiError::internal)?;
                    acquire_metadata_write_lock(&tx).await?;
                    let pending = load_pending_source_blob_deletions(&tx).await?;
                    let referenced_blob_keys = referenced_source_blob_keys(&tx).await?;
                    let (result, retained) = op(pending, &referenced_blob_keys)?;
                    save_pending_source_blob_deletions(&tx, &retained).await?;
                    tx.commit().await.map_err(ApiError::internal)?;
                    Ok(result)
                })
            }
            #[cfg(any(test, feature = "memory-metadata"))]
            MetadataStoreInner::Memory(_) => self.update(move |catalog| {
                let referenced_blob_keys = catalog
                    .repositories
                    .values()
                    .flat_map(|repo| repo.source_blobs())
                    .map(|blob| blob.object_key)
                    .collect::<BTreeSet<_>>();
                let pending = std::mem::take(&mut catalog.pending_source_blob_deletions);
                let (result, retained) = op(pending, &referenced_blob_keys)?;
                catalog.pending_source_blob_deletions = retained;
                Ok(result)
            }),
        }
    }
}

pub(super) fn queue_pending_source_blob_deletions(
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

pub(super) fn queue_pending_repo_storage_deletion(
    pending: &mut Vec<RepoStorageCleanup>,
    cleanup: RepoStorageCleanup,
) {
    let cleanup_repo_id = repo_id(&cleanup.owner_handle, &cleanup.repo_name);
    let already_queued = pending
        .iter()
        .any(|pending| repo_id(&pending.owner_handle, &pending.repo_name) == cleanup_repo_id);
    if !already_queued {
        pending.push(cleanup);
    }
}

pub(super) async fn load_pending_repo_storage_deletions<C>(
    conn: &C,
) -> Result<Vec<RepoStorageCleanup>, ApiError>
where
    C: ConnectionTrait,
{
    let metadata_lock = entities::metadata_lock::Entity::find_by_id(METADATA_LOCK_KEY.to_string())
        .one(conn)
        .await
        .map_err(ApiError::internal)?
        .ok_or_else(|| ApiError::internal_message("metadata lock row is missing"))?;
    decode_json::<Vec<RepoStorageCleanup>>(metadata_lock.pending_repo_storage_deletions)
}

pub(super) async fn save_pending_repo_storage_deletions<C>(
    conn: &C,
    pending_repo_storage_deletions: &[RepoStorageCleanup],
) -> Result<(), ApiError>
where
    C: ConnectionTrait,
{
    entities::metadata_lock::Entity::update_many()
        .filter(entities::metadata_lock::Column::Key.eq(METADATA_LOCK_KEY))
        .col_expr(
            entities::metadata_lock::Column::PendingRepoStorageDeletions,
            Expr::value(encode_json(&pending_repo_storage_deletions)?),
        )
        .exec(conn)
        .await
        .map_err(ApiError::internal)?;
    Ok(())
}

pub(super) async fn load_pending_source_blob_deletions<C>(
    conn: &C,
) -> Result<Vec<SourceBlob>, ApiError>
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

pub(super) async fn save_pending_source_blob_deletions<C>(
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

pub(super) fn remove_matching_pending_repo_storage_cleanup(
    cleanups: &mut Vec<RepoStorageCleanup>,
    cleanup_repo_id: &str,
) -> bool {
    let original_len = cleanups.len();
    cleanups
        .retain(|cleanup| repo_id(&cleanup.owner_handle, &cleanup.repo_name) != cleanup_repo_id);
    cleanups.len() != original_len
}

async fn live_repo_ids_for_cleanups<C>(
    conn: &C,
    pending: &[RepoStorageCleanup],
) -> Result<BTreeSet<String>, ApiError>
where
    C: ConnectionTrait,
{
    let cleanup_repo_ids = pending
        .iter()
        .map(|cleanup| repo_id(&cleanup.owner_handle, &cleanup.repo_name))
        .collect::<Vec<_>>();
    if cleanup_repo_ids.is_empty() {
        return Ok(BTreeSet::new());
    }

    let repositories = entities::repository::Entity::find()
        .filter(entities::repository::Column::Id.is_in(cleanup_repo_ids))
        .all(conn)
        .await
        .map_err(ApiError::internal)?;
    Ok(repositories.into_iter().map(|repo| repo.id).collect())
}

async fn referenced_source_blob_keys<C>(conn: &C) -> Result<BTreeSet<String>, ApiError>
where
    C: ConnectionTrait,
{
    let repositories = entities::repository::Entity::find()
        .order_by_asc(entities::repository::Column::Id)
        .all(conn)
        .await
        .map_err(ApiError::internal)?;
    let repositories = repositories_from_models(conn, repositories).await?;
    Ok(repositories
        .into_iter()
        .flat_map(|repo| repo.source_blobs())
        .map(|blob| blob.object_key)
        .collect())
}
