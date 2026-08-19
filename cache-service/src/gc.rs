use crate::AppState;
use scope_object_store::ObjectStore;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

const BATCH_SIZE: u64 = 100;
const RETRY_SECONDS: u64 = 5 * 60;

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
    let now = SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs();
    let retry_at = now
        .checked_add(RETRY_SECONDS)
        .ok_or_else(|| anyhow::anyhow!("cache retry timestamp overflow"))?;
    let caches = state.metadata.caches();
    caches.expire_references(now, BATCH_SIZE).await?;
    caches.expire_committed_uploads(now, BATCH_SIZE).await?;
    for upload in caches.expire_uploads(now, BATCH_SIZE).await? {
        match delete_object(state.object_store.clone(), upload.object_key.clone()).await {
            Ok(()) => caches.complete_upload_cleanup(&upload.upload_id).await?,
            Err(error) => {
                caches.retry_upload_cleanup(&upload.upload_id).await?;
                tracing::warn!(
                    upload_id = %upload.upload_id,
                    object_key = %upload.object_key,
                    %error,
                    "expired cache upload deletion will be retried"
                );
            }
        }
    }
    let deletions = caches.claim_deletions(now, retry_at, BATCH_SIZE).await?;
    for deletion in deletions {
        match delete_object(state.object_store.clone(), deletion.object_key.clone()).await {
            Ok(()) => {
                caches
                    .complete_deletion(&deletion.repository_id, &deletion.checksum_sha256)
                    .await?;
            }
            Err(error) => {
                caches
                    .fail_deletion(&deletion, retry_at, &error.to_string())
                    .await?;
            }
        }
    }
    Ok(())
}

async fn delete_object(
    store: std::sync::Arc<scope_object_store::S3ObjectStore>,
    object_key: String,
) -> anyhow::Result<()> {
    tokio::task::spawn_blocking(move || store.delete(&object_key)).await??;
    Ok(())
}
