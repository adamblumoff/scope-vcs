use super::{MetadataStore, entities};
use crate::domain::{
    projection::{SourceGraph, VisibilityEvent},
    store::{RepoStorageCleanup, SourceBlob, repo_id},
};
use crate::{error::ApiError, persistence::unix_now};
use sea_orm::{
    ActiveModelTrait, ColumnTrait, ConnectionTrait, DatabaseBackend, EntityTrait, FromQueryResult,
    IntoActiveModel, QueryFilter, QueryOrder, QuerySelect, Set, Statement, TransactionTrait,
    prelude::Json,
    sea_query::{Expr, LockType, OnConflict},
};
use std::future::Future;
use std::{collections::BTreeSet, sync::Arc};

const RETAINED_REPO_STORAGE_ERROR: &str = "repo storage cleanup retained after drain attempt";
const RETAINED_SOURCE_BLOB_ERROR: &str = "source blob cleanup retained after drain attempt";
const CLEANUP_BATCH_SIZE: u64 = 100;
const CLEANUP_CLAIM_SECONDS: i64 = 300;
const MAX_CLEANUP_RETRY_SECONDS: i64 = 3_600;

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

#[derive(FromQueryResult)]
struct RepositoryBlobReferencesRow {
    graph: Json,
    visibility_events: Json,
}

#[derive(FromQueryResult)]
struct RequestBlobReferenceRow {
    git_snapshot: Option<Json>,
}

pub struct RepoStorageCleanupBatch {
    pub pending: Vec<RepoStorageCleanup>,
    pub live_repo_ids: BTreeSet<String>,
    loaded: Vec<LoadedRepoStorageCleanup>,
}

pub struct SourceBlobCleanupBatch {
    pub pending: Vec<SourceBlob>,
    pub referenced_blob_keys: BTreeSet<String>,
    loaded: Vec<LoadedSourceBlobCleanup>,
}

pub struct RepoStorageCleanupClaim {
    generation: String,
    claim_until: i64,
}

impl MetadataStore {
    /// Serializes filesystem deletion and repository creation for one stable owner/name path.
    /// The session lock spans external I/O without holding a metadata transaction open.
    pub async fn with_repo_storage_lock<R, F, Fut>(
        &self,
        repo_id: &str,
        op: F,
    ) -> Result<R, ApiError>
    where
        F: FnOnce() -> Fut,
        Fut: Future<Output = Result<R, ApiError>>,
    {
        use sea_orm::sqlx::Connection;

        let schema = self
            .db
            .query_one(Statement::from_string(
                DatabaseBackend::Postgres,
                "SELECT current_schema() AS schema".to_string(),
            ))
            .await
            .map_err(ApiError::internal)?
            .ok_or_else(|| ApiError::internal_message("Postgres did not return its schema"))?
            .try_get::<String>("", "schema")
            .map_err(ApiError::internal)?;
        let database_url = self.postgres_database_url.as_deref().ok_or_else(|| {
            ApiError::internal_message("repository storage lock requires Postgres")
        })?;
        let connection = sea_orm::sqlx::PgConnection::connect(database_url)
            .await
            .map_err(ApiError::internal)?;
        let lock = sea_orm::sqlx::postgres::PgAdvisoryLock::new(format!(
            "scope:repo-storage:{schema}:{repo_id}"
        ));
        let guard = lock.acquire(connection).await.map_err(ApiError::internal)?;
        let result = op().await;
        let connection = guard.release_now().await.map_err(ApiError::internal)?;
        connection.close().await.map_err(ApiError::internal)?;
        result
    }

    pub async fn repository_exists(&self, repo_id: &str) -> Result<bool, ApiError> {
        entities::repository::Entity::find_by_id(repo_id.to_string())
            .one(self.db.as_ref())
            .await
            .map(|row| row.is_some())
            .map_err(ApiError::internal)
    }

    pub async fn pending_cleanup_queues(
        &self,
    ) -> Result<(Vec<RepoStorageCleanup>, Vec<SourceBlob>), ApiError> {
        Ok((
            load_pending_repo_storage_deletions(self.db.as_ref()).await?,
            load_pending_source_blob_deletions(self.db.as_ref()).await?,
        ))
    }

    pub async fn unreferenced_source_blobs(
        &self,
        blobs: Vec<SourceBlob>,
    ) -> Result<Vec<SourceBlob>, ApiError> {
        let referenced = referenced_source_blob_keys(self.db.as_ref()).await?;
        let mut unreferenced = std::collections::BTreeMap::new();
        for blob in blobs {
            if !referenced.contains(&blob.object_key) {
                unreferenced.entry(blob.object_key.clone()).or_insert(blob);
            }
        }
        Ok(unreferenced.into_values().collect())
    }

    pub async fn queue_pending_source_blob_deletions(
        &self,
        blobs: Vec<SourceBlob>,
    ) -> Result<(), ApiError> {
        if blobs.is_empty() {
            return Ok(());
        }

        let db = Arc::clone(&self.db);
        let tx = db.as_ref().begin().await.map_err(ApiError::internal)?;
        queue_pending_source_blob_deletion_rows(&tx, blobs).await?;
        tx.commit().await.map_err(ApiError::internal)?;
        Ok(())
    }

    pub async fn repo_storage_cleanup_batch(&self) -> Result<RepoStorageCleanupBatch, ApiError> {
        let db = Arc::clone(&self.db);
        let tx = db.as_ref().begin().await.map_err(ApiError::internal)?;
        let loaded = claim_pending_repo_storage_cleanup_rows(&tx).await?;
        let pending = loaded
            .iter()
            .map(|row| row.cleanup.clone())
            .collect::<Vec<_>>();
        let live_repo_ids = live_repo_ids_for_cleanups(&tx, &pending).await?;
        tx.commit().await.map_err(ApiError::internal)?;
        Ok(RepoStorageCleanupBatch {
            pending,
            live_repo_ids,
            loaded,
        })
    }

    pub async fn finish_repo_storage_cleanup(
        &self,
        batch: RepoStorageCleanupBatch,
        retained: &[RepoStorageCleanup],
    ) -> Result<(), ApiError> {
        let tx = self.db.begin().await.map_err(ApiError::internal)?;
        reconcile_repo_storage_cleanup_rows(&tx, &batch.loaded, retained, &batch.live_repo_ids)
            .await?;
        tx.commit().await.map_err(ApiError::internal)
    }

    pub async fn source_blob_cleanup_batch(&self) -> Result<SourceBlobCleanupBatch, ApiError> {
        let db = Arc::clone(&self.db);
        let tx = db.as_ref().begin().await.map_err(ApiError::internal)?;
        let loaded = claim_pending_source_blob_cleanup_rows(&tx).await?;
        let pending = loaded
            .iter()
            .map(|row| row.blob.clone())
            .collect::<Vec<_>>();
        let referenced_blob_keys = referenced_source_blob_keys(&tx).await?;
        tx.commit().await.map_err(ApiError::internal)?;
        Ok(SourceBlobCleanupBatch {
            pending,
            referenced_blob_keys,
            loaded,
        })
    }

    pub async fn finish_source_blob_cleanup(
        &self,
        batch: SourceBlobCleanupBatch,
        retained: &[SourceBlob],
    ) -> Result<(), ApiError> {
        let tx = self.db.begin().await.map_err(ApiError::internal)?;
        reconcile_source_blob_cleanup_rows(&tx, &batch.loaded, retained).await?;
        tx.commit().await.map_err(ApiError::internal)
    }
}

pub async fn queue_pending_repo_storage_cleanup_row<C>(
    conn: &C,
    cleanup: RepoStorageCleanup,
) -> Result<(), ApiError>
where
    C: ConnectionTrait,
{
    let now = unix_now()?;
    let generation = new_cleanup_generation()?;
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

pub async fn queue_pending_source_blob_deletion_rows<C>(
    conn: &C,
    blobs: impl IntoIterator<Item = SourceBlob>,
) -> Result<(), ApiError>
where
    C: ConnectionTrait,
{
    let now = unix_now()?;
    for blob in blobs {
        u64_to_i64(blob.size_bytes)?;
        let generation = new_cleanup_generation()?;
        entities::source_blob_cleanup_job::Entity::insert(
            entities::source_blob_cleanup_job::Model::from_domain(&blob, generation, now)?
                .into_active_model(),
        )
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
        .map_err(ApiError::internal)?;
    }
    Ok(())
}

pub async fn load_pending_repo_storage_deletions<C>(
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

async fn claim_pending_repo_storage_cleanup_rows<C>(
    conn: &C,
) -> Result<Vec<LoadedRepoStorageCleanup>, ApiError>
where
    C: ConnectionTrait,
{
    let now = u64_to_i64(unix_now()?)?;
    let claim_until = now
        .checked_add(CLEANUP_CLAIM_SECONDS)
        .ok_or_else(|| ApiError::internal_message("cleanup claim time exceeds i64 range"))?;
    let rows = entities::repo_storage_cleanup_job::Model::find_by_statement(
        Statement::from_sql_and_values(
            DatabaseBackend::Postgres,
            r#"
                UPDATE scope_repo_storage_cleanup_jobs AS job
                SET generation = md5(job.generation || ':' || txid_current()::text),
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
            [now.into(), claim_until.into(), CLEANUP_BATCH_SIZE.into()],
        ),
    )
    .all(conn)
    .await
    .map_err(ApiError::internal)?;
    Ok(rows
        .into_iter()
        .map(|row| LoadedRepoStorageCleanup {
            generation: row.generation.clone(),
            cleanup: row.into_domain(),
        })
        .collect())
}

#[cfg(any(test, feature = "local-dev", feature = "test-support"))]
pub async fn save_pending_repo_storage_deletions<C>(
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

pub async fn pending_repo_storage_cleanup_exists<C>(
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

/// Claims a pending repository storage deletion while a repository is being recreated.
///
/// The caller must hold the repository aggregate lock. The lease keeps cleanup workers and
/// competing creators from deleting or recreating the same storage while external cleanup runs.
pub async fn claim_pending_repo_storage_cleanup<C>(
    conn: &C,
    cleanup_repo_id: &str,
) -> Result<Option<RepoStorageCleanupClaim>, ApiError>
where
    C: ConnectionTrait,
{
    let now_i64 = u64_to_i64(unix_now()?)?;
    let Some(row) =
        entities::repo_storage_cleanup_job::Entity::find_by_id(cleanup_repo_id.to_string())
            .filter(entities::repo_storage_cleanup_job::Column::CompletedAtUnix.is_null())
            .lock(LockType::Update)
            .one(conn)
            .await
            .map_err(ApiError::internal)?
    else {
        return Ok(None);
    };
    if row.next_run_at_unix > now_i64 {
        return Err(ApiError::conflict(
            "repository storage cleanup is already in progress; retry",
        ));
    }
    let claim_until = now_i64
        .checked_add(CLEANUP_CLAIM_SECONDS)
        .ok_or_else(|| ApiError::internal_message("cleanup claim time exceeds i64 range"))?;
    let generation = new_cleanup_generation()?;
    let mut active = row.into_active_model();
    active.generation = Set(generation.clone());
    active.next_run_at_unix = Set(claim_until);
    active.updated_at_unix = Set(now_i64);
    active.update(conn).await.map_err(ApiError::internal)?;
    Ok(Some(RepoStorageCleanupClaim {
        generation,
        claim_until,
    }))
}

pub async fn complete_claimed_repo_storage_cleanup<C>(
    conn: &C,
    cleanup_repo_id: &str,
    claim: &RepoStorageCleanupClaim,
) -> Result<(), ApiError>
where
    C: ConnectionTrait,
{
    let now = u64_to_i64(unix_now()?)?;
    if now >= claim.claim_until {
        return Err(ApiError::conflict(
            "repository storage cleanup claim expired during creation; retry",
        ));
    }
    let result = complete_pending_repo_storage_cleanup_update(
        conn,
        cleanup_repo_id,
        &claim.generation,
        now,
        Some(claim.claim_until),
    )
    .await?;
    if result.rows_affected == 1 {
        Ok(())
    } else {
        Err(ApiError::conflict(
            "repository storage cleanup changed during creation; retry",
        ))
    }
}

pub async fn load_pending_source_blob_deletions<C>(conn: &C) -> Result<Vec<SourceBlob>, ApiError>
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
        .map(|blob| {
            let generation = blob.generation.clone();
            Ok(LoadedSourceBlobCleanup {
                generation,
                blob: blob.try_into_domain()?,
            })
        })
        .collect::<Result<Vec<_>, ApiError>>()?;
    Ok(pending)
}

async fn claim_pending_source_blob_cleanup_rows<C>(
    conn: &C,
) -> Result<Vec<LoadedSourceBlobCleanup>, ApiError>
where
    C: ConnectionTrait,
{
    let now = u64_to_i64(unix_now()?)?;
    let claim_until = now
        .checked_add(CLEANUP_CLAIM_SECONDS)
        .ok_or_else(|| ApiError::internal_message("cleanup claim time exceeds i64 range"))?;
    let rows = entities::source_blob_cleanup_job::Model::find_by_statement(
        Statement::from_sql_and_values(
            DatabaseBackend::Postgres,
            r#"
                UPDATE scope_source_blob_cleanup_jobs AS job
                SET generation = md5(job.generation || ':' || txid_current()::text),
                    next_run_at_unix = $2,
                    updated_at_unix = $1
                FROM (
                    SELECT object_key
                    FROM scope_source_blob_cleanup_jobs
                    WHERE completed_at_unix IS NULL AND next_run_at_unix <= $1
                    ORDER BY next_run_at_unix, object_key
                    FOR UPDATE SKIP LOCKED
                    LIMIT $3
                ) AS claimed
                WHERE job.object_key = claimed.object_key
                RETURNING job.*
            "#,
            [now.into(), claim_until.into(), CLEANUP_BATCH_SIZE.into()],
        ),
    )
    .all(conn)
    .await
    .map_err(ApiError::internal)?;
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

#[cfg(any(test, feature = "local-dev", feature = "test-support"))]
pub async fn save_pending_source_blob_deletions<C>(
    conn: &C,
    pending_source_blob_deletions: &[SourceBlob],
) -> Result<(), ApiError>
where
    C: ConnectionTrait,
{
    queue_pending_source_blob_deletion_rows(conn, pending_source_blob_deletions.iter().cloned())
        .await
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
        model.attempts.checked_add(1).ok_or_else(|| {
            ApiError::internal_message("repository cleanup attempt count exceeds i32 range")
        })?
    } else {
        model.attempts
    };
    let next_run_at = if last_error.is_some() {
        next_cleanup_retry_at(now_i64, attempts)?
    } else {
        now_i64
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
            entities::repo_storage_cleanup_job::Column::NextRunAtUnix,
            Expr::value(next_run_at),
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
    let attempts = model.attempts.checked_add(1).ok_or_else(|| {
        ApiError::internal_message("source blob cleanup attempt count exceeds i32 range")
    })?;
    let next_run_at = next_cleanup_retry_at(now_i64, attempts)?;
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
            entities::source_blob_cleanup_job::Column::NextRunAtUnix,
            Expr::value(next_run_at),
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

pub async fn complete_pending_repo_storage_cleanup_at<C>(
    conn: &C,
    cleanup_repo_id: &str,
    generation: &str,
    now_i64: i64,
) -> Result<(), ApiError>
where
    C: ConnectionTrait,
{
    complete_pending_repo_storage_cleanup_update(conn, cleanup_repo_id, generation, now_i64, None)
        .await?;
    Ok(())
}

async fn complete_pending_repo_storage_cleanup_update<C>(
    conn: &C,
    cleanup_repo_id: &str,
    generation: &str,
    now_i64: i64,
    claim_until: Option<i64>,
) -> Result<sea_orm::UpdateResult, ApiError>
where
    C: ConnectionTrait,
{
    let mut update = entities::repo_storage_cleanup_job::Entity::update_many()
        .filter(entities::repo_storage_cleanup_job::Column::RepoId.eq(cleanup_repo_id.to_string()))
        .filter(entities::repo_storage_cleanup_job::Column::Generation.eq(generation.to_string()))
        .filter(entities::repo_storage_cleanup_job::Column::CompletedAtUnix.is_null());
    if let Some(claim_until) = claim_until {
        update = update
            .filter(entities::repo_storage_cleanup_job::Column::NextRunAtUnix.eq(claim_until));
    }
    update
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
        .map_err(ApiError::internal)
}

pub async fn complete_pending_source_blob_cleanup_at<C>(
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
    let mut keys = entities::repository_git_snapshot::Entity::find()
        .select_only()
        .column(entities::repository_git_snapshot::Column::ObjectKey)
        .into_tuple::<String>()
        .all(conn)
        .await
        .map_err(ApiError::internal)?
        .into_iter()
        .collect::<BTreeSet<_>>();

    let repositories = entities::repository::Entity::find()
        .select_only()
        .column(entities::repository::Column::Graph)
        .column(entities::repository::Column::VisibilityEvents)
        .into_model::<RepositoryBlobReferencesRow>()
        .all(conn)
        .await
        .map_err(ApiError::internal)?;
    for repository in repositories {
        let graph =
            serde_json::from_value::<SourceGraph>(repository.graph).map_err(ApiError::internal)?;
        for change in graph.commits.into_iter().flat_map(|commit| commit.changes) {
            keys.extend(change.old_content.into_iter().map(|blob| blob.object_key));
            keys.extend(change.new_content.into_iter().map(|blob| blob.object_key));
        }
        let visibility_events =
            serde_json::from_value::<Vec<VisibilityEvent>>(repository.visibility_events)
                .map_err(ApiError::internal)?;
        keys.extend(
            visibility_events
                .into_iter()
                .filter_map(|event| event.current_content)
                .map(|blob| blob.object_key),
        );
    }

    let requests = entities::request::Entity::find()
        .select_only()
        .column(entities::request::Column::GitSnapshot)
        .into_model::<RequestBlobReferenceRow>()
        .all(conn)
        .await
        .map_err(ApiError::internal)?;
    for request in requests {
        if let Some(snapshot) = request
            .git_snapshot
            .map(serde_json::from_value::<SourceBlob>)
            .transpose()
            .map_err(ApiError::internal)?
        {
            keys.insert(snapshot.object_key);
        }
    }
    Ok(keys)
}

fn u64_to_i64(value: u64) -> Result<i64, ApiError> {
    i64::try_from(value).map_err(|_| ApiError::internal_message("timestamp exceeds i64 range"))
}

fn next_cleanup_retry_at(now: i64, attempts: i32) -> Result<i64, ApiError> {
    let exponent = attempts
        .checked_sub(2)
        .filter(|value| *value >= -1)
        .ok_or_else(|| ApiError::internal_message("cleanup attempt count must be positive"))?;
    if exponent == -1 {
        return Ok(now);
    }
    let exponent = u32::try_from(exponent.min(10))
        .map_err(|_| ApiError::internal_message("cleanup retry exponent cannot be negative"))?;
    let delay = 5_i64
        .checked_mul(2_i64.pow(exponent))
        .unwrap_or(MAX_CLEANUP_RETRY_SECONDS)
        .min(MAX_CLEANUP_RETRY_SECONDS);
    now.checked_add(delay)
        .ok_or_else(|| ApiError::internal_message("cleanup retry time exceeds i64 range"))
}

fn new_cleanup_generation() -> Result<String, ApiError> {
    let mut bytes = [0_u8; 16];
    getrandom::fill(&mut bytes).map_err(|error| {
        ApiError::internal_message(format!("failed to generate cleanup row token: {error}"))
    })?;
    Ok(hex::encode(bytes))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cleanup_retry_backoff_is_bounded() {
        assert_eq!(next_cleanup_retry_at(100, 1).unwrap(), 100);
        assert_eq!(next_cleanup_retry_at(100, 2).unwrap(), 105);
        assert_eq!(next_cleanup_retry_at(100, 3).unwrap(), 110);
        assert_eq!(next_cleanup_retry_at(100, 20).unwrap(), 3_700);
        assert!(next_cleanup_retry_at(100, 0).is_err());
    }
}
