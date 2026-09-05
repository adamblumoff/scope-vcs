use crate::AppState;
use futures_util::{StreamExt, stream};
use scope_object_store::ObjectStore;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

const BATCH_SIZE: u64 = 100;
const RETRY_SECONDS: u64 = 5 * 60;
const DELETE_CONCURRENCY: usize = 8;
const SWEEP_BUDGET: Duration = Duration::from_secs(30);

pub(crate) fn start(state: AppState) {
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_secs(60));
        loop {
            interval.tick().await;
            if let Err(error) = reconcile(&state).await {
                tracing::error!(%error, "cache reconciliation failed");
            }
        }
    });
}

async fn reconcile(state: &AppState) -> anyhow::Result<()> {
    let started = std::time::Instant::now();
    let mut batches = 0;
    let mut clean_uploads = true;
    loop {
        let more = reconcile_batch(state, &mut clean_uploads).await?;
        batches += 1;
        // Finish each claimed batch before stopping so every deletion gets its acknowledgement.
        if !more || started.elapsed() >= SWEEP_BUDGET {
            tracing::info!(
                batches,
                more,
                elapsed_ms = started.elapsed().as_millis(),
                "cache reconciliation finished"
            );
            return Ok(());
        }
    }
}

async fn reconcile_batch(state: &AppState, clean_uploads: &mut bool) -> anyhow::Result<bool> {
    let now = SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs();
    let retry_at = now
        .checked_add(RETRY_SECONDS)
        .ok_or_else(|| anyhow::anyhow!("cache retry timestamp overflow"))?;
    let caches = state.metadata.caches();
    let orphans = caches
        .claim_orphan_uploads(now, retry_at, BATCH_SIZE)
        .await?;
    let mut more = orphans.len() as u64 == BATCH_SIZE;
    for (upload, result) in delete_objects(state.object_store.clone(), orphans, |upload| {
        upload.object_key.clone()
    })
    .await
    {
        match result {
            Ok(()) => {
                caches
                    .complete_orphan_upload_cleanup(&upload.object_key)
                    .await?
            }
            Err(error) => {
                caches
                    .fail_orphan_upload_cleanup(&upload.object_key, retry_at, &error.to_string())
                    .await?
            }
        }
    }
    more |= caches.expire_references(now, BATCH_SIZE).await? == BATCH_SIZE;
    more |= caches.expire_committed_uploads(now, BATCH_SIZE).await? == BATCH_SIZE;
    let uploads = if *clean_uploads {
        caches.expire_uploads(now, BATCH_SIZE).await?
    } else {
        Vec::new()
    };
    let mut more_uploads = uploads.len() as u64 == BATCH_SIZE;
    for (upload, result) in delete_objects(state.object_store.clone(), uploads, |upload| {
        upload.object_key.clone()
    })
    .await
    {
        match result {
            Ok(()) => caches.complete_upload_cleanup(&upload.upload_id).await?,
            Err(error) => {
                caches.retry_upload_cleanup(&upload.upload_id).await?;
                more_uploads = false;
                *clean_uploads = false;
                tracing::warn!(upload_id = %upload.upload_id, object_key = %upload.object_key, %error, "expired cache upload deletion will be retried");
            }
        }
    }
    more |= more_uploads;
    let deletions = caches.claim_deletions(now, retry_at, BATCH_SIZE).await?;
    more |= deletions.len() as u64 == BATCH_SIZE;
    for (deletion, result) in delete_objects(state.object_store.clone(), deletions, |deletion| {
        deletion.object_key.clone()
    })
    .await
    {
        match result {
            Ok(()) => {
                caches
                    .complete_deletion(&deletion.repository_id, &deletion.checksum_sha256)
                    .await?;
            }
            Err(error) => {
                caches
                    .fail_deletion(&deletion, retry_at, &error.to_string())
                    .await?
            }
        }
    }
    Ok(more)
}

async fn delete_objects<T>(
    store: std::sync::Arc<scope_object_store::S3ObjectStore>,
    objects: Vec<T>,
    object_key: impl Fn(&T) -> String,
) -> Vec<(T, anyhow::Result<()>)> {
    stream::iter(objects.into_iter().map(|object| {
        let store = store.clone();
        let key = object_key(&object);
        async move { (object, delete_object(store, key).await) }
    }))
    .buffer_unordered(DELETE_CONCURRENCY)
    .collect()
    .await
}

async fn delete_object(
    store: std::sync::Arc<scope_object_store::S3ObjectStore>,
    object_key: String,
) -> anyhow::Result<()> {
    tokio::task::spawn_blocking(move || store.delete(&object_key)).await??;
    Ok(())
}

#[cfg(test)]
mod tests;
