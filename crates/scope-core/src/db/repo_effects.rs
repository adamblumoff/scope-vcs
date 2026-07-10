use super::{
    cleanup_queue::{
        queue_pending_repo_storage_cleanup_row, queue_pending_source_blob_deletion_rows,
    },
    repository_rows::save_repository_delta,
};
use crate::{
    domain::{
        repo_actions::{RepoEffect, RepoEffects},
        store::StoredRepository,
    },
    error::ApiError,
};
use sea_orm::ConnectionTrait;

pub async fn save_repo_mutation<C>(
    conn: &C,
    before: &StoredRepository,
    repo: &StoredRepository,
    effects: &RepoEffects,
) -> Result<(), ApiError>
where
    C: ConnectionTrait,
{
    save_repository_delta(conn, before, repo).await?;
    save_repo_effects(conn, effects).await
}

pub async fn save_repo_effects<C>(conn: &C, effects: &RepoEffects) -> Result<(), ApiError>
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
