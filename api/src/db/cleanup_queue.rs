use super::{METADATA_LOCK_KEY, decode_json, encode_json, entities};
use crate::domain::store::{RepoStorageCleanup, SourceBlob, repo_id};
use crate::error::ApiError;
use sea_orm::{ColumnTrait, ConnectionTrait, EntityTrait, QueryFilter, sea_query::Expr};

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
