#[cfg(any(test, feature = "memory-metadata"))]
use super::cleanup_queue::{
    queue_pending_repo_storage_deletion, queue_pending_source_blob_deletions,
};
use super::{
    cleanup_queue::{
        queue_pending_repo_storage_cleanup_row, queue_pending_source_blob_deletion_rows,
    },
    repository_rows::save_repository_row,
};
#[cfg(any(test, feature = "memory-metadata"))]
use crate::domain::store::AppCatalog;
use crate::{
    domain::{
        repo_actions::{RepoEffect, RepoEffects},
        store::StoredRepository,
    },
    error::ApiError,
};
use sea_orm::ConnectionTrait;

pub(super) async fn save_repo_mutation<C>(
    conn: &C,
    repo: &StoredRepository,
    effects: &RepoEffects,
) -> Result<(), ApiError>
where
    C: ConnectionTrait,
{
    save_repository_row(conn, repo).await?;
    save_repo_effects(conn, effects).await
}

pub(super) async fn save_repo_effects<C>(conn: &C, effects: &RepoEffects) -> Result<(), ApiError>
where
    C: ConnectionTrait,
{
    if effects.is_empty() {
        return Ok(());
    }

    for effect in effects.iter() {
        match effect {
            RepoEffect::DeleteRepoStorage(cleanup) => {
                queue_pending_repo_storage_cleanup_row(conn, cleanup.clone()).await?;
            }
            RepoEffect::DeleteSourceBlobs(blobs) => {
                queue_pending_source_blob_deletion_rows(conn, blobs.clone()).await?;
            }
        }
    }

    Ok(())
}

#[cfg(any(test, feature = "memory-metadata"))]
pub(super) fn apply_repo_effects(catalog: &mut AppCatalog, effects: RepoEffects) {
    for effect in effects.iter() {
        match effect {
            RepoEffect::DeleteRepoStorage(cleanup) => {
                queue_pending_repo_storage_deletion(
                    &mut catalog.pending_repo_storage_deletions,
                    cleanup.clone(),
                );
            }
            RepoEffect::DeleteSourceBlobs(blobs) => {
                queue_pending_source_blob_deletions(
                    &mut catalog.pending_source_blob_deletions,
                    blobs.clone(),
                );
            }
        }
    }
}
