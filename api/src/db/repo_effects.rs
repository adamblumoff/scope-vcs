use super::{
    cleanup_queue::{
        load_pending_repo_storage_deletions, load_pending_source_blob_deletions,
        queue_pending_repo_storage_deletion, queue_pending_source_blob_deletions,
        save_pending_repo_storage_deletions, save_pending_source_blob_deletions,
    },
    repository_rows::save_repository_row,
};
#[cfg(test)]
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
                let mut pending_repo_storage_deletions =
                    load_pending_repo_storage_deletions(conn).await?;
                queue_pending_repo_storage_deletion(
                    &mut pending_repo_storage_deletions,
                    cleanup.clone(),
                );
                save_pending_repo_storage_deletions(conn, &pending_repo_storage_deletions).await?;
            }
            RepoEffect::DeleteSourceBlobs(blobs) => {
                let mut pending_source_blob_deletions =
                    load_pending_source_blob_deletions(conn).await?;
                queue_pending_source_blob_deletions(
                    &mut pending_source_blob_deletions,
                    blobs.clone(),
                );
                save_pending_source_blob_deletions(conn, &pending_source_blob_deletions).await?;
            }
        }
    }

    Ok(())
}

#[cfg(test)]
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
