use super::{
    mapping::u64_to_i64,
    revalidation::live_repo_ids_for_cleanups,
    types::{
        LoadedRepoStorageCleanup, LoadedSourceBlobCleanup, RepoStorageCleanupBatch,
        RepoStorageCleanupClaim, SourceBlobCleanupBatch,
    },
};
use crate::{
    db::{CleanupStore, GeneratedIdKind, GeneratedIdSource, entities, generated_ids::generate_id},
    error::PostgresError,
};
use sea_orm::{
    ActiveModelTrait, ColumnTrait, ConnectionTrait, DatabaseBackend, EntityTrait, FromQueryResult,
    IntoActiveModel, QueryFilter, QuerySelect, Set, Statement, TransactionTrait,
    sea_query::LockType,
};
use std::sync::Arc;

const CLEANUP_BATCH_SIZE: u64 = 100;
const CLEANUP_CLAIM_SECONDS: i64 = 300;

impl CleanupStore {
    pub async fn repo_storage_cleanup_batch(
        &self,
        now_unix: u64,
        generated_ids: &dyn GeneratedIdSource,
    ) -> Result<RepoStorageCleanupBatch, PostgresError> {
        let db = Arc::clone(&self.db);
        let tx = db.as_ref().begin().await.map_err(PostgresError::internal)?;
        let loaded = claim_pending_repo_storage_cleanup_rows(&tx, now_unix, generated_ids).await?;
        let pending = loaded
            .iter()
            .map(|row| row.cleanup.clone())
            .collect::<Vec<_>>();
        let live_repo_ids = live_repo_ids_for_cleanups(&tx, &pending).await?;
        tx.commit().await.map_err(PostgresError::internal)?;
        Ok(RepoStorageCleanupBatch {
            pending,
            live_repo_ids,
            loaded,
        })
    }
    pub async fn source_blob_cleanup_batch(
        &self,
        now_unix: u64,
        generated_ids: &dyn GeneratedIdSource,
    ) -> Result<SourceBlobCleanupBatch, PostgresError> {
        let db = Arc::clone(&self.db);
        let tx = db.as_ref().begin().await.map_err(PostgresError::internal)?;
        let loaded = claim_pending_source_blob_cleanup_rows(&tx, now_unix, generated_ids).await?;
        let pending = loaded
            .iter()
            .map(|row| row.blob.clone())
            .collect::<Vec<_>>();
        tx.commit().await.map_err(PostgresError::internal)?;
        Ok(SourceBlobCleanupBatch { pending, loaded })
    }
}

async fn claim_pending_repo_storage_cleanup_rows<C>(
    conn: &C,
    now_unix: u64,
    generated_ids: &dyn GeneratedIdSource,
) -> Result<Vec<LoadedRepoStorageCleanup>, PostgresError>
where
    C: ConnectionTrait,
{
    let now = u64_to_i64(now_unix)?;
    let generation = generate_id(generated_ids, GeneratedIdKind::CleanupGeneration)?;
    let claim_until = now
        .checked_add(CLEANUP_CLAIM_SECONDS)
        .ok_or_else(|| PostgresError::internal_message("cleanup claim time exceeds i64 range"))?;
    let rows = entities::repo_storage_cleanup_job::Model::find_by_statement(
        Statement::from_sql_and_values(
            DatabaseBackend::Postgres,
            r#"
                UPDATE scope_repo_storage_cleanup_jobs AS job
                SET generation = $4,
                    next_run_at_unix = $2,
                    updated_at_unix = $1
                FROM (
                    SELECT repo_id
                    FROM scope_repo_storage_cleanup_jobs
                    WHERE completed_at_unix IS NULL AND next_run_at_unix <= $1
                    ORDER BY next_run_at_unix, repo_id
                    FOR UPDATE SKIP LOCKED
                    LIMIT $3
                ) AS claimed
                WHERE job.repo_id = claimed.repo_id
                RETURNING job.*
            "#,
            [
                now.into(),
                claim_until.into(),
                CLEANUP_BATCH_SIZE.into(),
                generation.into(),
            ],
        ),
    )
    .all(conn)
    .await
    .map_err(PostgresError::internal)?;
    Ok(rows
        .into_iter()
        .map(|row| LoadedRepoStorageCleanup {
            generation: row.generation.clone(),
            cleanup: row.into_domain(),
        })
        .collect())
}

pub async fn claim_pending_repo_storage_cleanup<C>(
    conn: &C,
    cleanup_repo_id: &str,
    now_unix: u64,
    generated_ids: &dyn GeneratedIdSource,
) -> Result<Option<RepoStorageCleanupClaim>, PostgresError>
where
    C: ConnectionTrait,
{
    let now_i64 = u64_to_i64(now_unix)?;
    let Some(row) =
        entities::repo_storage_cleanup_job::Entity::find_by_id(cleanup_repo_id.to_string())
            .filter(entities::repo_storage_cleanup_job::Column::CompletedAtUnix.is_null())
            .lock(LockType::Update)
            .one(conn)
            .await
            .map_err(PostgresError::internal)?
    else {
        return Ok(None);
    };
    if row.next_run_at_unix > now_i64 {
        return Err(PostgresError::conflict(
            "repository storage cleanup is already in progress; retry",
        ));
    }
    let claim_until = now_i64
        .checked_add(CLEANUP_CLAIM_SECONDS)
        .ok_or_else(|| PostgresError::internal_message("cleanup claim time exceeds i64 range"))?;
    let generation = generate_id(generated_ids, GeneratedIdKind::CleanupGeneration)?;
    let mut active = row.into_active_model();
    active.generation = Set(generation.clone());
    active.next_run_at_unix = Set(claim_until);
    active.updated_at_unix = Set(now_i64);
    active.update(conn).await.map_err(PostgresError::internal)?;
    Ok(Some(RepoStorageCleanupClaim {
        generation,
        claim_until,
    }))
}
async fn claim_pending_source_blob_cleanup_rows<C>(
    conn: &C,
    now_unix: u64,
    generated_ids: &dyn GeneratedIdSource,
) -> Result<Vec<LoadedSourceBlobCleanup>, PostgresError>
where
    C: ConnectionTrait,
{
    let now = u64_to_i64(now_unix)?;
    let generation = generate_id(generated_ids, GeneratedIdKind::CleanupGeneration)?;
    let claim_until = now
        .checked_add(CLEANUP_CLAIM_SECONDS)
        .ok_or_else(|| PostgresError::internal_message("cleanup claim time exceeds i64 range"))?;
    let rows = entities::source_blob_cleanup_job::Model::find_by_statement(
        Statement::from_sql_and_values(
            DatabaseBackend::Postgres,
            r#"
                UPDATE scope_orphan_object_jobs AS job
                SET generation = $4,
                    next_run_at_unix = $2,
                    updated_at_unix = $1
                FROM (
                    SELECT object_key
                    FROM scope_orphan_object_jobs
                    WHERE completed_at_unix IS NULL AND next_run_at_unix <= $1
                    ORDER BY next_run_at_unix, object_key
                    FOR UPDATE SKIP LOCKED
                    LIMIT $3
                ) AS claimed
                WHERE job.object_key = claimed.object_key
                RETURNING job.*
            "#,
            [
                now.into(),
                claim_until.into(),
                CLEANUP_BATCH_SIZE.into(),
                generation.into(),
            ],
        ),
    )
    .all(conn)
    .await
    .map_err(PostgresError::internal)?;
    rows.into_iter()
        .map(|row| {
            let generation = row.generation.clone();
            Ok(LoadedSourceBlobCleanup {
                generation,
                blob: row.try_into_domain()?,
            })
        })
        .collect()
}
