use super::{
    MetadataStore, MetadataStoreInner, acquire_metadata_write_lock, entities,
    repositories_from_models, run_api_db_on,
};
use crate::domain::store::{RepoStorageCleanup, SourceBlob, repo_id};
use crate::{error::ApiError, persistence::unix_now};
use sea_orm::{
    ColumnTrait, ConnectionTrait, EntityTrait, IntoActiveModel, QueryFilter, QueryOrder,
    TransactionTrait,
    sea_query::{Expr, OnConflict},
};
use std::{collections::BTreeSet, sync::Arc};

const RETAINED_REPO_STORAGE_ERROR: &str = "repo storage cleanup retained after drain attempt";
const RETAINED_SOURCE_BLOB_ERROR: &str = "source blob cleanup retained after drain attempt";

#[derive(Clone)]
struct LoadedRepoStorageCleanup {
    cleanup: RepoStorageCleanup,
    generation: String,
}

#[derive(Clone)]
struct LoadedSourceBlobCleanup {
    blob: SourceBlob,
    generation: String,
}

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
                    queue_pending_source_blob_deletion_rows(&tx, blobs).await?;
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
                    let loaded = load_pending_repo_storage_cleanup_rows(&tx).await?;
                    let pending = loaded
                        .iter()
                        .map(|row| row.cleanup.clone())
                        .collect::<Vec<_>>();
                    let live_repo_ids = live_repo_ids_for_cleanups(&tx, &pending).await?;
                    let (result, retained) = op(pending.clone(), &live_repo_ids)?;
                    reconcile_repo_storage_cleanup_rows(&tx, &loaded, &retained, &live_repo_ids)
                        .await?;
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
                    let loaded = load_pending_source_blob_cleanup_rows(&tx).await?;
                    let pending = loaded
                        .iter()
                        .map(|row| row.blob.clone())
                        .collect::<Vec<_>>();
                    let referenced_blob_keys = referenced_source_blob_keys(&tx).await?;
                    let (result, retained) = op(pending.clone(), &referenced_blob_keys)?;
                    reconcile_source_blob_cleanup_rows(&tx, &loaded, &retained).await?;
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

#[cfg(any(test, feature = "memory-metadata"))]
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

#[cfg(any(test, feature = "memory-metadata"))]
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

pub(super) async fn queue_pending_repo_storage_cleanup_row<C>(
    conn: &C,
    cleanup: RepoStorageCleanup,
) -> Result<(), ApiError>
where
    C: ConnectionTrait,
{
    let now = unix_now()?;
    let generation = new_cleanup_generation()?;
    entities::repo_storage_cleanup_job::Entity::insert(
        entities::repo_storage_cleanup_job::Model::from_domain(&cleanup, generation, now)
            .into_active_model(),
    )
    .on_conflict(
        OnConflict::column(entities::repo_storage_cleanup_job::Column::RepoId)
            .update_columns([
                entities::repo_storage_cleanup_job::Column::Generation,
                entities::repo_storage_cleanup_job::Column::OwnerHandle,
                entities::repo_storage_cleanup_job::Column::RepoName,
                entities::repo_storage_cleanup_job::Column::Attempts,
                entities::repo_storage_cleanup_job::Column::NextRunAtUnix,
                entities::repo_storage_cleanup_job::Column::LastError,
                entities::repo_storage_cleanup_job::Column::CompletedAtUnix,
                entities::repo_storage_cleanup_job::Column::UpdatedAtUnix,
            ])
            .to_owned(),
    )
    .exec(conn)
    .await
    .map_err(ApiError::internal)?;
    Ok(())
}

pub(super) async fn queue_pending_source_blob_deletion_rows<C>(
    conn: &C,
    blobs: impl IntoIterator<Item = SourceBlob>,
) -> Result<(), ApiError>
where
    C: ConnectionTrait,
{
    let now = unix_now()?;
    for blob in blobs {
        u64_to_i64(blob.size_bytes)?;
        usize_to_i64(blob.line_count)?;
        let generation = new_cleanup_generation()?;
        entities::source_blob_cleanup_job::Entity::insert(
            entities::source_blob_cleanup_job::Model::from_domain(&blob, generation, now)
                .into_active_model(),
        )
        .on_conflict(
            OnConflict::column(entities::source_blob_cleanup_job::Column::ObjectKey)
                .update_columns([
                    entities::source_blob_cleanup_job::Column::Generation,
                    entities::source_blob_cleanup_job::Column::Sha256,
                    entities::source_blob_cleanup_job::Column::GitOid,
                    entities::source_blob_cleanup_job::Column::SizeBytes,
                    entities::source_blob_cleanup_job::Column::LineCount,
                    entities::source_blob_cleanup_job::Column::Attempts,
                    entities::source_blob_cleanup_job::Column::NextRunAtUnix,
                    entities::source_blob_cleanup_job::Column::LastError,
                    entities::source_blob_cleanup_job::Column::CompletedAtUnix,
                    entities::source_blob_cleanup_job::Column::UpdatedAtUnix,
                ])
                .to_owned(),
        )
        .exec(conn)
        .await
        .map_err(ApiError::internal)?;
    }
    Ok(())
}

pub(super) async fn load_pending_repo_storage_deletions<C>(
    conn: &C,
) -> Result<Vec<RepoStorageCleanup>, ApiError>
where
    C: ConnectionTrait,
{
    let pending = load_pending_repo_storage_cleanup_rows(conn)
        .await?
        .into_iter()
        .map(|row| row.cleanup)
        .collect::<Vec<_>>();
    Ok(pending)
}

async fn load_pending_repo_storage_cleanup_rows<C>(
    conn: &C,
) -> Result<Vec<LoadedRepoStorageCleanup>, ApiError>
where
    C: ConnectionTrait,
{
    let pending = entities::repo_storage_cleanup_job::Entity::find()
        .filter(entities::repo_storage_cleanup_job::Column::CompletedAtUnix.is_null())
        .order_by_asc(entities::repo_storage_cleanup_job::Column::RepoId)
        .all(conn)
        .await
        .map_err(ApiError::internal)?
        .into_iter()
        .map(|cleanup| LoadedRepoStorageCleanup {
            generation: cleanup.generation.clone(),
            cleanup: cleanup.into_domain(),
        })
        .collect::<Vec<_>>();
    Ok(pending)
}

#[cfg(test)]
pub(super) async fn save_pending_repo_storage_deletions<C>(
    conn: &C,
    pending_repo_storage_deletions: &[RepoStorageCleanup],
) -> Result<(), ApiError>
where
    C: ConnectionTrait,
{
    for cleanup in pending_repo_storage_deletions {
        queue_pending_repo_storage_cleanup_row(conn, cleanup.clone()).await?;
    }
    Ok(())
}

pub(super) async fn pending_repo_storage_cleanup_exists<C>(
    conn: &C,
    cleanup_repo_id: &str,
) -> Result<bool, ApiError>
where
    C: ConnectionTrait,
{
    let row = entities::repo_storage_cleanup_job::Entity::find_by_id(cleanup_repo_id.to_string())
        .filter(entities::repo_storage_cleanup_job::Column::CompletedAtUnix.is_null())
        .one(conn)
        .await
        .map_err(ApiError::internal)?;
    Ok(row.is_some())
}

pub(super) async fn complete_pending_repo_storage_cleanup<C>(
    conn: &C,
    cleanup_repo_id: &str,
) -> Result<(), ApiError>
where
    C: ConnectionTrait,
{
    let now_i64 = u64_to_i64(unix_now()?)?;
    entities::repo_storage_cleanup_job::Entity::update_many()
        .filter(entities::repo_storage_cleanup_job::Column::RepoId.eq(cleanup_repo_id.to_string()))
        .filter(entities::repo_storage_cleanup_job::Column::CompletedAtUnix.is_null())
        .col_expr(
            entities::repo_storage_cleanup_job::Column::CompletedAtUnix,
            Expr::value(now_i64),
        )
        .col_expr(
            entities::repo_storage_cleanup_job::Column::UpdatedAtUnix,
            Expr::value(now_i64),
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
    let pending = load_pending_source_blob_cleanup_rows(conn)
        .await?
        .into_iter()
        .map(|row| row.blob)
        .collect::<Vec<_>>();
    Ok(pending)
}

async fn load_pending_source_blob_cleanup_rows<C>(
    conn: &C,
) -> Result<Vec<LoadedSourceBlobCleanup>, ApiError>
where
    C: ConnectionTrait,
{
    let pending = entities::source_blob_cleanup_job::Entity::find()
        .filter(entities::source_blob_cleanup_job::Column::CompletedAtUnix.is_null())
        .order_by_asc(entities::source_blob_cleanup_job::Column::ObjectKey)
        .all(conn)
        .await
        .map_err(ApiError::internal)?
        .into_iter()
        .map(|blob| LoadedSourceBlobCleanup {
            generation: blob.generation.clone(),
            blob: blob.into_domain(),
        })
        .collect::<Vec<_>>();
    Ok(pending)
}

#[cfg(test)]
pub(super) async fn save_pending_source_blob_deletions<C>(
    conn: &C,
    pending_source_blob_deletions: &[SourceBlob],
) -> Result<(), ApiError>
where
    C: ConnectionTrait,
{
    queue_pending_source_blob_deletion_rows(conn, pending_source_blob_deletions.iter().cloned())
        .await
}

#[cfg(any(test, feature = "memory-metadata"))]
pub(super) fn remove_matching_pending_repo_storage_cleanup(
    cleanups: &mut Vec<RepoStorageCleanup>,
    cleanup_repo_id: &str,
) -> bool {
    let original_len = cleanups.len();
    cleanups
        .retain(|cleanup| repo_id(&cleanup.owner_handle, &cleanup.repo_name) != cleanup_repo_id);
    cleanups.len() != original_len
}

async fn reconcile_repo_storage_cleanup_rows<C>(
    conn: &C,
    loaded: &[LoadedRepoStorageCleanup],
    retained: &[RepoStorageCleanup],
    live_repo_ids: &BTreeSet<String>,
) -> Result<(), ApiError>
where
    C: ConnectionTrait,
{
    let retained_repo_ids = retained
        .iter()
        .map(|cleanup| repo_id(&cleanup.owner_handle, &cleanup.repo_name))
        .collect::<BTreeSet<_>>();
    let now_i64 = u64_to_i64(unix_now()?)?;
    for loaded_cleanup in loaded {
        let cleanup = &loaded_cleanup.cleanup;
        let cleanup_repo_id = repo_id(&cleanup.owner_handle, &cleanup.repo_name);
        if retained_repo_ids.contains(&cleanup_repo_id) {
            let last_error = (!live_repo_ids.contains(&cleanup_repo_id))
                .then(|| RETAINED_REPO_STORAGE_ERROR.to_string());
            mark_repo_storage_cleanup_retained(
                conn,
                &cleanup_repo_id,
                &loaded_cleanup.generation,
                last_error,
                now_i64,
            )
            .await?;
        } else {
            complete_pending_repo_storage_cleanup_at(
                conn,
                &cleanup_repo_id,
                &loaded_cleanup.generation,
                now_i64,
            )
            .await?;
        }
    }
    for cleanup in retained {
        if !loaded.iter().any(|loaded| {
            repo_id(&loaded.cleanup.owner_handle, &loaded.cleanup.repo_name)
                == repo_id(&cleanup.owner_handle, &cleanup.repo_name)
        }) {
            queue_pending_repo_storage_cleanup_row(conn, cleanup.clone()).await?;
        }
    }
    Ok(())
}

async fn reconcile_source_blob_cleanup_rows<C>(
    conn: &C,
    loaded: &[LoadedSourceBlobCleanup],
    retained: &[SourceBlob],
) -> Result<(), ApiError>
where
    C: ConnectionTrait,
{
    let retained_object_keys = retained
        .iter()
        .map(|blob| blob.object_key.clone())
        .collect::<BTreeSet<_>>();
    let now_i64 = u64_to_i64(unix_now()?)?;
    for loaded_blob in loaded {
        let blob = &loaded_blob.blob;
        if retained_object_keys.contains(&blob.object_key) {
            mark_source_blob_cleanup_retained(
                conn,
                &blob.object_key,
                &loaded_blob.generation,
                now_i64,
            )
            .await?;
        } else {
            complete_pending_source_blob_cleanup_at(
                conn,
                &blob.object_key,
                &loaded_blob.generation,
                now_i64,
            )
            .await?;
        }
    }
    for blob in retained {
        if !loaded
            .iter()
            .any(|loaded| loaded.blob.object_key == blob.object_key)
        {
            queue_pending_source_blob_deletion_rows(conn, [blob.clone()]).await?;
        }
    }
    Ok(())
}

async fn mark_repo_storage_cleanup_retained<C>(
    conn: &C,
    cleanup_repo_id: &str,
    generation: &str,
    last_error: Option<String>,
    now_i64: i64,
) -> Result<(), ApiError>
where
    C: ConnectionTrait,
{
    let Some(model) =
        entities::repo_storage_cleanup_job::Entity::find_by_id(cleanup_repo_id.to_string())
            .one(conn)
            .await
            .map_err(ApiError::internal)?
    else {
        return Ok(());
    };
    if model.generation != generation || model.completed_at_unix.is_some() {
        return Ok(());
    }
    let attempts = if last_error.is_some() {
        model.attempts.saturating_add(1)
    } else {
        model.attempts
    };
    entities::repo_storage_cleanup_job::Entity::update_many()
        .filter(entities::repo_storage_cleanup_job::Column::RepoId.eq(cleanup_repo_id.to_string()))
        .filter(entities::repo_storage_cleanup_job::Column::Generation.eq(generation.to_string()))
        .filter(entities::repo_storage_cleanup_job::Column::CompletedAtUnix.is_null())
        .col_expr(
            entities::repo_storage_cleanup_job::Column::Attempts,
            Expr::value(attempts),
        )
        .col_expr(
            entities::repo_storage_cleanup_job::Column::LastError,
            Expr::value(last_error),
        )
        .col_expr(
            entities::repo_storage_cleanup_job::Column::UpdatedAtUnix,
            Expr::value(now_i64),
        )
        .exec(conn)
        .await
        .map_err(ApiError::internal)?;
    Ok(())
}

async fn mark_source_blob_cleanup_retained<C>(
    conn: &C,
    object_key: &str,
    generation: &str,
    now_i64: i64,
) -> Result<(), ApiError>
where
    C: ConnectionTrait,
{
    let Some(model) = entities::source_blob_cleanup_job::Entity::find_by_id(object_key.to_string())
        .one(conn)
        .await
        .map_err(ApiError::internal)?
    else {
        return Ok(());
    };
    if model.generation != generation || model.completed_at_unix.is_some() {
        return Ok(());
    }
    let attempts = model.attempts.saturating_add(1);
    entities::source_blob_cleanup_job::Entity::update_many()
        .filter(entities::source_blob_cleanup_job::Column::ObjectKey.eq(object_key.to_string()))
        .filter(entities::source_blob_cleanup_job::Column::Generation.eq(generation.to_string()))
        .filter(entities::source_blob_cleanup_job::Column::CompletedAtUnix.is_null())
        .col_expr(
            entities::source_blob_cleanup_job::Column::Attempts,
            Expr::value(attempts),
        )
        .col_expr(
            entities::source_blob_cleanup_job::Column::LastError,
            Expr::value(Some(RETAINED_SOURCE_BLOB_ERROR.to_string())),
        )
        .col_expr(
            entities::source_blob_cleanup_job::Column::UpdatedAtUnix,
            Expr::value(now_i64),
        )
        .exec(conn)
        .await
        .map_err(ApiError::internal)?;
    Ok(())
}

pub(super) async fn complete_pending_repo_storage_cleanup_at<C>(
    conn: &C,
    cleanup_repo_id: &str,
    generation: &str,
    now_i64: i64,
) -> Result<(), ApiError>
where
    C: ConnectionTrait,
{
    entities::repo_storage_cleanup_job::Entity::update_many()
        .filter(entities::repo_storage_cleanup_job::Column::RepoId.eq(cleanup_repo_id.to_string()))
        .filter(entities::repo_storage_cleanup_job::Column::Generation.eq(generation.to_string()))
        .filter(entities::repo_storage_cleanup_job::Column::CompletedAtUnix.is_null())
        .col_expr(
            entities::repo_storage_cleanup_job::Column::CompletedAtUnix,
            Expr::value(now_i64),
        )
        .col_expr(
            entities::repo_storage_cleanup_job::Column::LastError,
            Expr::value(Option::<String>::None),
        )
        .col_expr(
            entities::repo_storage_cleanup_job::Column::UpdatedAtUnix,
            Expr::value(now_i64),
        )
        .exec(conn)
        .await
        .map_err(ApiError::internal)?;
    Ok(())
}

pub(super) async fn complete_pending_source_blob_cleanup_at<C>(
    conn: &C,
    object_key: &str,
    generation: &str,
    now_i64: i64,
) -> Result<(), ApiError>
where
    C: ConnectionTrait,
{
    entities::source_blob_cleanup_job::Entity::update_many()
        .filter(entities::source_blob_cleanup_job::Column::ObjectKey.eq(object_key.to_string()))
        .filter(entities::source_blob_cleanup_job::Column::Generation.eq(generation.to_string()))
        .filter(entities::source_blob_cleanup_job::Column::CompletedAtUnix.is_null())
        .col_expr(
            entities::source_blob_cleanup_job::Column::CompletedAtUnix,
            Expr::value(now_i64),
        )
        .col_expr(
            entities::source_blob_cleanup_job::Column::LastError,
            Expr::value(Option::<String>::None),
        )
        .col_expr(
            entities::source_blob_cleanup_job::Column::UpdatedAtUnix,
            Expr::value(now_i64),
        )
        .exec(conn)
        .await
        .map_err(ApiError::internal)?;
    Ok(())
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

fn u64_to_i64(value: u64) -> Result<i64, ApiError> {
    if value > i64::MAX as u64 {
        return Err(ApiError::internal_message("timestamp exceeds i64 range"));
    }
    Ok(value as i64)
}

fn usize_to_i64(value: usize) -> Result<i64, ApiError> {
    if value > i64::MAX as usize {
        return Err(ApiError::internal_message("line count exceeds i64 range"));
    }
    Ok(value as i64)
}

fn new_cleanup_generation() -> Result<String, ApiError> {
    let mut bytes = [0_u8; 16];
    getrandom::fill(&mut bytes).map_err(|error| {
        ApiError::internal_message(format!("failed to generate cleanup row token: {error}"))
    })?;
    Ok(hex::encode(bytes))
}
