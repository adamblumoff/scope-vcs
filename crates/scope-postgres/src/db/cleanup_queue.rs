use super::{
    CleanupStore, GeneratedIdKind, GeneratedIdSource, RepositoryStore, entities,
    generated_ids::generate_id,
};
use crate::error::PostgresError;
use scope_domain::{
    content_ref::ContentRef,
    store::{RepoStorageCleanup, SourceBlob, repo_id},
};
use sea_orm::{
    ActiveModelTrait, ColumnTrait, ConnectionTrait, DatabaseBackend, EntityTrait, FromQueryResult,
    IntoActiveModel, QueryFilter, QueryOrder, QuerySelect, Set, Statement, TransactionTrait,
    sea_query::{Expr, LockType, OnConflict},
};
use std::future::Future;
use std::{collections::BTreeSet, sync::Arc};

const RETAINED_REPO_STORAGE_ERROR: &str = "repo storage cleanup retained after drain attempt";
const RETAINED_SOURCE_BLOB_ERROR: &str = "source blob cleanup retained after drain attempt";
const CLEANUP_BATCH_SIZE: u64 = 100;
const CLEANUP_CLAIM_SECONDS: i64 = 300;
const MAX_CLEANUP_RETRY_SECONDS: i64 = 3_600;
#[cfg(not(feature = "test-support"))]
const SOURCE_BLOB_DELETE_GRACE_SECONDS: u64 = 300;
#[cfg(feature = "test-support")]
const SOURCE_BLOB_DELETE_GRACE_SECONDS: u64 = 0;

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

pub struct RepoStorageCleanupBatch {
    pub pending: Vec<RepoStorageCleanup>,
    pub live_repo_ids: BTreeSet<String>,
    loaded: Vec<LoadedRepoStorageCleanup>,
}

pub struct SourceBlobCleanupBatch {
    pub pending: Vec<SourceBlob>,
    pub referenced_content_refs: BTreeSet<ContentRef>,
    loaded: Vec<LoadedSourceBlobCleanup>,
}

pub struct RepoStorageCleanupClaim {
    generation: String,
    claim_until: i64,
}

impl RepositoryStore {
    /// Serializes filesystem deletion and repository creation for one stable owner/name path.
    /// The session lock spans external I/O without holding a metadata transaction open.
    pub async fn with_repo_storage_lock<R, F, Fut, E>(&self, repo_id: &str, op: F) -> Result<R, E>
    where
        F: FnOnce() -> Fut,
        Fut: Future<Output = Result<R, E>>,
        E: From<PostgresError>,
    {
        let schema = self
            .db
            .query_one(Statement::from_string(
                DatabaseBackend::Postgres,
                "SELECT current_schema() AS schema".to_string(),
            ))
            .await
            .map_err(PostgresError::internal)?
            .ok_or_else(|| PostgresError::internal_message("Postgres did not return its schema"))?
            .try_get::<String>("", "schema")
            .map_err(PostgresError::internal)?;
        let connection = self
            .db
            .get_postgres_connection_pool()
            .acquire()
            .await
            .map_err(PostgresError::internal)?;
        let lock = sea_orm::sqlx::postgres::PgAdvisoryLock::new(format!(
            "scope:repo-storage:{schema}:{repo_id}"
        ));
        let guard = lock
            .acquire(connection)
            .await
            .map_err(PostgresError::internal)?;
        let result = op().await;
        guard.release_now().await.map_err(PostgresError::internal)?;
        result
    }

    pub async fn repository_exists(&self, repo_id: &str) -> Result<bool, PostgresError> {
        entities::repository::Entity::find_by_id(repo_id.to_string())
            .one(self.db.as_ref())
            .await
            .map(|row| row.is_some())
            .map_err(PostgresError::internal)
    }
}

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

    pub async fn finish_repo_storage_cleanup(
        &self,
        batch: RepoStorageCleanupBatch,
        retained: &[RepoStorageCleanup],
        now_unix: u64,
        generated_ids: &dyn GeneratedIdSource,
    ) -> Result<(), PostgresError> {
        let tx = self.db.begin().await.map_err(PostgresError::internal)?;
        reconcile_repo_storage_cleanup_rows(
            &tx,
            &batch.loaded,
            retained,
            &batch.live_repo_ids,
            now_unix,
            generated_ids,
        )
        .await?;
        tx.commit().await.map_err(PostgresError::internal)
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
        let referenced_content_refs = referenced_content_refs(&tx).await?;
        tx.commit().await.map_err(PostgresError::internal)?;
        Ok(SourceBlobCleanupBatch {
            pending,
            referenced_content_refs,
            loaded,
        })
    }

    pub async fn finish_source_blob_cleanup(
        &self,
        batch: SourceBlobCleanupBatch,
        retained: &[SourceBlob],
        now_unix: u64,
        generated_ids: &dyn GeneratedIdSource,
    ) -> Result<(), PostgresError> {
        let tx = self.db.begin().await.map_err(PostgresError::internal)?;
        reconcile_source_blob_cleanup_rows(&tx, &batch.loaded, retained, now_unix, generated_ids)
            .await?;
        tx.commit().await.map_err(PostgresError::internal)
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

pub(super) async fn queue_pending_repo_storage_cleanup_row_at<C>(
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

async fn queue_pending_source_blob_deletion_rows_at<C>(
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

#[cfg(any(test, feature = "local-dev", feature = "test-support"))]
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
            &super::generated_ids::test_generated_id,
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

/// Claims a pending repository storage deletion while a repository is being recreated.
///
/// The caller must hold the repository aggregate lock. The lease keeps cleanup workers and
/// competing creators from deleting or recreating the same storage while external cleanup runs.
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

pub async fn complete_claimed_repo_storage_cleanup<C>(
    conn: &C,
    cleanup_repo_id: &str,
    claim: &RepoStorageCleanupClaim,
    now_unix: u64,
) -> Result<(), PostgresError>
where
    C: ConnectionTrait,
{
    let now = u64_to_i64(now_unix)?;
    if now >= claim.claim_until {
        return Err(PostgresError::conflict(
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
        Err(PostgresError::conflict(
            "repository storage cleanup changed during creation; retry",
        ))
    }
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

#[cfg(any(test, feature = "local-dev", feature = "test-support"))]
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
        &super::generated_ids::test_generated_id,
    )
    .await
}

async fn reconcile_repo_storage_cleanup_rows<C>(
    conn: &C,
    loaded: &[LoadedRepoStorageCleanup],
    retained: &[RepoStorageCleanup],
    live_repo_ids: &BTreeSet<String>,
    now_unix: u64,
    generated_ids: &dyn GeneratedIdSource,
) -> Result<(), PostgresError>
where
    C: ConnectionTrait,
{
    let retained_repo_ids = retained
        .iter()
        .map(|cleanup| repo_id(&cleanup.owner_handle, &cleanup.repo_name))
        .collect::<BTreeSet<_>>();
    let now_i64 = u64_to_i64(now_unix)?;
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
            queue_pending_repo_storage_cleanup_row_at(
                conn,
                cleanup.clone(),
                now_unix,
                generated_ids,
            )
            .await?;
        }
    }
    Ok(())
}

async fn reconcile_source_blob_cleanup_rows<C>(
    conn: &C,
    loaded: &[LoadedSourceBlobCleanup],
    retained: &[SourceBlob],
    now_unix: u64,
    generated_ids: &dyn GeneratedIdSource,
) -> Result<(), PostgresError>
where
    C: ConnectionTrait,
{
    let retained_content_refs = retained
        .iter()
        .map(|blob| blob.content_ref.clone())
        .collect::<BTreeSet<_>>();
    let now_i64 = u64_to_i64(now_unix)?;
    for loaded_blob in loaded {
        let blob = &loaded_blob.blob;
        if retained_content_refs.contains(&blob.content_ref) {
            mark_source_blob_cleanup_retained(
                conn,
                &blob.content_ref,
                &loaded_blob.generation,
                now_i64,
            )
            .await?;
        } else {
            complete_pending_source_blob_cleanup_at(
                conn,
                &blob.content_ref,
                &loaded_blob.generation,
                now_i64,
            )
            .await?;
        }
    }
    for blob in retained {
        if !loaded
            .iter()
            .any(|loaded| loaded.blob.content_ref == blob.content_ref)
        {
            queue_pending_source_blob_deletion_rows_at(
                conn,
                [blob.clone()],
                now_unix,
                generated_ids,
            )
            .await?;
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
) -> Result<(), PostgresError>
where
    C: ConnectionTrait,
{
    let Some(model) =
        entities::repo_storage_cleanup_job::Entity::find_by_id(cleanup_repo_id.to_string())
            .one(conn)
            .await
            .map_err(PostgresError::internal)?
    else {
        return Ok(());
    };
    if model.generation != generation || model.completed_at_unix.is_some() {
        return Ok(());
    }
    let attempts = if last_error.is_some() {
        model.attempts.checked_add(1).ok_or_else(|| {
            PostgresError::internal_message("repository cleanup attempt count exceeds i32 range")
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
        .map_err(PostgresError::internal)?;
    Ok(())
}

async fn mark_source_blob_cleanup_retained<C>(
    conn: &C,
    content_ref: &ContentRef,
    generation: &str,
    now_i64: i64,
) -> Result<(), PostgresError>
where
    C: ConnectionTrait,
{
    let encoded_content_ref = encode_content_ref(content_ref)?;
    let Some(model) =
        entities::source_blob_cleanup_job::Entity::find_by_id(encoded_content_ref.clone())
            .one(conn)
            .await
            .map_err(PostgresError::internal)?
    else {
        return Ok(());
    };
    if model.generation != generation || model.completed_at_unix.is_some() {
        return Ok(());
    }
    let attempts = model.attempts.checked_add(1).ok_or_else(|| {
        PostgresError::internal_message("source blob cleanup attempt count exceeds i32 range")
    })?;
    let next_run_at = next_cleanup_retry_at(now_i64, attempts)?;
    entities::source_blob_cleanup_job::Entity::update_many()
        .filter(entities::source_blob_cleanup_job::Column::ObjectKey.eq(encoded_content_ref))
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
        .map_err(PostgresError::internal)?;
    Ok(())
}

pub async fn complete_pending_repo_storage_cleanup_at<C>(
    conn: &C,
    cleanup_repo_id: &str,
    generation: &str,
    now_i64: i64,
) -> Result<(), PostgresError>
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
) -> Result<sea_orm::UpdateResult, PostgresError>
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
        .map_err(PostgresError::internal)
}

pub async fn complete_pending_source_blob_cleanup_at<C>(
    conn: &C,
    content_ref: &ContentRef,
    generation: &str,
    now_i64: i64,
) -> Result<(), PostgresError>
where
    C: ConnectionTrait,
{
    let encoded_content_ref = encode_content_ref(content_ref)?;
    entities::source_blob_cleanup_job::Entity::update_many()
        .filter(entities::source_blob_cleanup_job::Column::ObjectKey.eq(encoded_content_ref))
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
        .map_err(PostgresError::internal)?;
    Ok(())
}

async fn live_repo_ids_for_cleanups<C>(
    conn: &C,
    pending: &[RepoStorageCleanup],
) -> Result<BTreeSet<String>, PostgresError>
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
        .map_err(PostgresError::internal)?;
    Ok(repositories.into_iter().map(|repo| repo.id).collect())
}

async fn referenced_content_refs<C>(conn: &C) -> Result<BTreeSet<ContentRef>, PostgresError>
where
    C: ConnectionTrait,
{
    super::object_references::referenced_content_refs(conn).await
}

fn encode_content_ref(content_ref: &ContentRef) -> Result<String, PostgresError> {
    serde_json::to_string(content_ref).map_err(PostgresError::internal)
}

fn u64_to_i64(value: u64) -> Result<i64, PostgresError> {
    i64::try_from(value).map_err(|_| PostgresError::internal_message("timestamp exceeds i64 range"))
}

fn next_cleanup_retry_at(now: i64, attempts: i32) -> Result<i64, PostgresError> {
    let exponent = attempts
        .checked_sub(2)
        .filter(|value| *value >= -1)
        .ok_or_else(|| PostgresError::internal_message("cleanup attempt count must be positive"))?;
    if exponent == -1 {
        return Ok(now);
    }
    let exponent = u32::try_from(exponent.min(10)).map_err(|_| {
        PostgresError::internal_message("cleanup retry exponent cannot be negative")
    })?;
    let delay = 5_i64
        .checked_mul(2_i64.pow(exponent))
        .unwrap_or(MAX_CLEANUP_RETRY_SECONDS)
        .min(MAX_CLEANUP_RETRY_SECONDS);
    now.checked_add(delay)
        .ok_or_else(|| PostgresError::internal_message("cleanup retry time exceeds i64 range"))
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
