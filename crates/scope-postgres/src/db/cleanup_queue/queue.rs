use super::{
    mapping::u64_to_i64,
    types::{LoadedRepoStorageCleanup, LoadedSourceBlobCleanup},
};
use crate::{
    db::{CleanupStore, GeneratedIdKind, GeneratedIdSource, entities, generated_ids::generate_id},
    error::PostgresError,
};
use scope_domain::{content::SourceBlob, repo_actions::RepoStorageCleanup};
use sea_orm::{
    ColumnTrait, ConnectionTrait, EntityTrait, IntoActiveModel, QueryFilter, QueryOrder, Set,
    TransactionTrait, sea_query::OnConflict,
};
use std::sync::Arc;

#[cfg(not(feature = "test-support"))]
pub(crate) const SOURCE_BLOB_DELETE_GRACE_SECONDS: u64 = 600;
#[cfg(feature = "test-support")]
pub(crate) const SOURCE_BLOB_DELETE_GRACE_SECONDS: u64 = 0;

impl CleanupStore {
    pub async fn pending_cleanup_queues(
        &self,
    ) -> Result<(Vec<RepoStorageCleanup>, Vec<SourceBlob>), PostgresError> {
        Ok((
            load_pending_repo_storage_deletions(self.db.as_ref()).await?,
            load_pending_source_blob_deletions(self.db.as_ref()).await?,
        ))
    }

    pub async fn queue_pending_source_blob_deletions(
        &self,
        blobs: Vec<SourceBlob>,
        now_unix: u64,
        generated_ids: &dyn GeneratedIdSource,
    ) -> Result<(), PostgresError> {
        if blobs.is_empty() {
            return Ok(());
        }

        let db = Arc::clone(&self.db);
        let tx = db.as_ref().begin().await.map_err(PostgresError::internal)?;
        queue_pending_source_blob_deletion_rows_at(&tx, blobs, now_unix, generated_ids).await?;
        tx.commit().await.map_err(PostgresError::internal)?;
        Ok(())
    }
}

pub async fn queue_pending_repo_storage_cleanup_row<C>(
    conn: &C,
    cleanup: RepoStorageCleanup,
    now_unix: u64,
    generated_ids: &dyn GeneratedIdSource,
) -> Result<(), PostgresError>
where
    C: ConnectionTrait,
{
    queue_pending_repo_storage_cleanup_row_at(conn, cleanup, now_unix, generated_ids).await
}

pub(crate) async fn queue_pending_repo_storage_cleanup_row_at<C>(
    conn: &C,
    cleanup: RepoStorageCleanup,
    now: u64,
    generated_ids: &dyn GeneratedIdSource,
) -> Result<(), PostgresError>
where
    C: ConnectionTrait,
{
    let generation = generate_id(generated_ids, GeneratedIdKind::CleanupGeneration)?;
    entities::repo_storage_cleanup_job::Entity::insert(
        entities::repo_storage_cleanup_job::Model::from_domain(&cleanup, generation, now)?
            .into_active_model(),
    )
    .on_conflict(
        OnConflict::column(entities::repo_storage_cleanup_job::Column::RepoId)
            .update_columns([
                entities::repo_storage_cleanup_job::Column::Generation,
                entities::repo_storage_cleanup_job::Column::OwnerHandle,
                entities::repo_storage_cleanup_job::Column::RepoName,
                entities::repo_storage_cleanup_job::Column::IncarnationId,
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
    .map_err(PostgresError::internal)?;
    Ok(())
}

pub async fn queue_pending_source_blob_deletion_rows<C>(
    conn: &C,
    blobs: impl IntoIterator<Item = SourceBlob>,
    now_unix: u64,
    generated_ids: &dyn GeneratedIdSource,
) -> Result<(), PostgresError>
where
    C: ConnectionTrait,
{
    queue_pending_source_blob_deletion_rows_at(conn, blobs, now_unix, generated_ids).await
}

pub(super) async fn queue_pending_source_blob_deletion_rows_at<C>(
    conn: &C,
    blobs: impl IntoIterator<Item = SourceBlob>,
    now: u64,
    generated_ids: &dyn GeneratedIdSource,
) -> Result<(), PostgresError>
where
    C: ConnectionTrait,
{
    let first_attempt = now
        .checked_add(SOURCE_BLOB_DELETE_GRACE_SECONDS)
        .ok_or_else(|| {
            PostgresError::internal_message("source blob cleanup time exceeds u64 range")
        })?;
    let first_attempt = u64_to_i64(first_attempt)?;
    for blob in blobs {
        u64_to_i64(blob.size_bytes)?;
        let generation = generate_id(generated_ids, GeneratedIdKind::CleanupGeneration)?;
        let mut cleanup =
            entities::source_blob_cleanup_job::Model::from_domain(&blob, generation, now)?
                .into_active_model();
        cleanup.next_run_at_unix = Set(first_attempt);
        entities::source_blob_cleanup_job::Entity::insert(cleanup)
            .on_conflict(
                OnConflict::column(entities::source_blob_cleanup_job::Column::ObjectKey)
                    .update_columns([
                        entities::source_blob_cleanup_job::Column::Generation,
                        entities::source_blob_cleanup_job::Column::Sha256,
                        entities::source_blob_cleanup_job::Column::GitOid,
                        entities::source_blob_cleanup_job::Column::SizeBytes,
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
            .map_err(PostgresError::internal)?;
    }
    Ok(())
}

pub async fn load_pending_repo_storage_deletions<C>(
    conn: &C,
) -> Result<Vec<RepoStorageCleanup>, PostgresError>
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
) -> Result<Vec<LoadedRepoStorageCleanup>, PostgresError>
where
    C: ConnectionTrait,
{
    let pending = entities::repo_storage_cleanup_job::Entity::find()
        .filter(entities::repo_storage_cleanup_job::Column::CompletedAtUnix.is_null())
        .order_by_asc(entities::repo_storage_cleanup_job::Column::RepoId)
        .all(conn)
        .await
        .map_err(PostgresError::internal)?
        .into_iter()
        .map(|cleanup| LoadedRepoStorageCleanup {
            generation: cleanup.generation.clone(),
            cleanup: cleanup.into_domain(),
        })
        .collect::<Vec<_>>();
    Ok(pending)
}
#[cfg(any(
    test,
    feature = "local-dev",
    feature = "smoke-seed",
    feature = "test-support"
))]
pub async fn save_pending_repo_storage_deletions<C>(
    conn: &C,
    pending_repo_storage_deletions: &[RepoStorageCleanup],
    now_unix: u64,
) -> Result<(), PostgresError>
where
    C: ConnectionTrait,
{
    for cleanup in pending_repo_storage_deletions {
        queue_pending_repo_storage_cleanup_row(
            conn,
            cleanup.clone(),
            now_unix,
            &crate::db::generated_ids::test_generated_id,
        )
        .await?;
    }
    Ok(())
}

pub async fn pending_repo_storage_cleanup_exists<C>(
    conn: &C,
    cleanup_repo_id: &str,
) -> Result<bool, PostgresError>
where
    C: ConnectionTrait,
{
    let row = entities::repo_storage_cleanup_job::Entity::find_by_id(cleanup_repo_id.to_string())
        .filter(entities::repo_storage_cleanup_job::Column::CompletedAtUnix.is_null())
        .one(conn)
        .await
        .map_err(PostgresError::internal)?;
    Ok(row.is_some())
}

pub async fn load_pending_source_blob_deletions<C>(
    conn: &C,
) -> Result<Vec<SourceBlob>, PostgresError>
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
) -> Result<Vec<LoadedSourceBlobCleanup>, PostgresError>
where
    C: ConnectionTrait,
{
    let pending = entities::source_blob_cleanup_job::Entity::find()
        .filter(entities::source_blob_cleanup_job::Column::CompletedAtUnix.is_null())
        .order_by_asc(entities::source_blob_cleanup_job::Column::ObjectKey)
        .all(conn)
        .await
        .map_err(PostgresError::internal)?
        .into_iter()
        .map(|blob| {
            let generation = blob.generation.clone();
            Ok(LoadedSourceBlobCleanup {
                generation,
                blob: blob.try_into_domain()?,
            })
        })
        .collect::<Result<Vec<_>, PostgresError>>()?;
    Ok(pending)
}
#[cfg(any(
    test,
    feature = "local-dev",
    feature = "smoke-seed",
    feature = "test-support"
))]
pub async fn save_pending_source_blob_deletions<C>(
    conn: &C,
    pending_source_blob_deletions: &[SourceBlob],
    now_unix: u64,
) -> Result<(), PostgresError>
where
    C: ConnectionTrait,
{
    queue_pending_source_blob_deletion_rows(
        conn,
        pending_source_blob_deletions.iter().cloned(),
        now_unix,
        &crate::db::generated_ids::test_generated_id,
    )
    .await
}
