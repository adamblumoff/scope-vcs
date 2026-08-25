use scope_domain::content::SourceBlob;
use scope_object_store::{ObjectStore, ObjectStoreError, object_key};
use scope_postgres::{
    db::{GeneratedIdSource, MetadataStore, cleanup_queue::types::SourceBlobCleanupDecision},
    error::PostgresError,
};
use std::collections::BTreeMap;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SourceBlobCleanupReport {
    pub attempted: usize,
    pub deleted: usize,
    pub retained: usize,
    pub skipped_referenced: usize,
    pub skipped_stale_claim: usize,
    pub failed_object_deletes: Vec<SourceBlobCleanupFailure>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SourceBlobCleanupFailure {
    pub blob: SourceBlob,
    pub error: ObjectStoreError,
}

pub async fn drain_source_blob_cleanup(
    metadata: &MetadataStore,
    object_store: &dyn ObjectStore,
    now_unix: u64,
    generated_ids: &dyn GeneratedIdSource,
) -> Result<SourceBlobCleanupReport, PostgresError> {
    let cleanup = metadata.cleanup();
    let batch = cleanup
        .source_blob_cleanup_batch(now_unix, generated_ids)
        .await?;
    let mut pending_by_key = BTreeMap::new();
    for blob in &batch.pending {
        pending_by_key
            .entry(object_key(blob))
            .or_insert_with(|| blob.clone());
    }

    let mut report = SourceBlobCleanupReport::default();
    let mut retained = Vec::new();
    for blob in pending_by_key.values() {
        let fence = metadata
            .acquire_content_ref_fence(std::slice::from_ref(&blob.content_ref))
            .await?;
        match cleanup.source_blob_cleanup_decision(&batch, blob).await? {
            SourceBlobCleanupDecision::Delete => {
                report.attempted += 1;
                match object_store.delete(&object_key(blob)) {
                    Ok(()) => report.deleted += 1,
                    Err(error) => {
                        retained.push(blob.clone());
                        report.failed_object_deletes.push(SourceBlobCleanupFailure {
                            blob: blob.clone(),
                            error,
                        });
                    }
                }
            }
            SourceBlobCleanupDecision::Referenced => report.skipped_referenced += 1,
            SourceBlobCleanupDecision::StaleClaim => report.skipped_stale_claim += 1,
        }
        fence.release().await;
    }
    report.retained = retained.len();
    cleanup
        .finish_source_blob_cleanup(batch, &retained, now_unix, generated_ids)
        .await?;
    Ok(report)
}
