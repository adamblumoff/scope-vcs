use super::{
    mapping::{encode_content_ref, u64_to_i64},
    queue::{
        queue_pending_repo_storage_cleanup_row_at, queue_pending_source_blob_deletion_rows_at,
    },
    types::{
        LoadedRepoStorageCleanup, LoadedSourceBlobCleanup, RepoStorageCleanupBatch,
        RepoStorageCleanupClaim, SourceBlobCleanupBatch,
    },
};
use crate::{
    db::{CleanupStore, GeneratedIdSource, entities},
    error::PostgresError,
};
use scope_domain::{
    content::SourceBlob, content_ref::ContentRef, repo_actions::RepoStorageCleanup,
    repository::repo_id,
};
use sea_orm::{
    ColumnTrait, ConnectionTrait, EntityTrait, QueryFilter, TransactionTrait, sea_query::Expr,
};
use std::collections::BTreeSet;

const RETAINED_REPO_STORAGE_ERROR: &str = "repo storage cleanup retained after drain attempt";
const RETAINED_SOURCE_BLOB_ERROR: &str = "source blob cleanup retained after drain attempt";
const MAX_CLEANUP_RETRY_SECONDS: i64 = 3_600;

impl CleanupStore {
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
