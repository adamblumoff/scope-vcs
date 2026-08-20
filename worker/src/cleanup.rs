use crate::{
    health::WorkerHealth,
    settings::{WorkerRole, WorkerSettings},
};
use scope_object_store::ObjectStore;
use scope_postgres::db::MetadataStore;
use std::{collections::BTreeMap, sync::Arc};

pub(crate) async fn run(
    metadata: MetadataStore,
    object_store: Arc<dyn ObjectStore>,
    settings: WorkerSettings,
    health: WorkerHealth,
) -> anyhow::Result<()> {
    loop {
        if !super::schema_ready_or_wait(&metadata, &health).await {
            return Ok(());
        }
        match drain_orphan_objects(&metadata, object_store.as_ref()).await {
            Ok(summary) => {
                if summary.attempted > 0 {
                    tracing::info!(
                        attempted = summary.attempted,
                        deleted = summary.deleted,
                        retained = summary.retained,
                        "processed orphan object jobs"
                    );
                }
            }
            Err(error) => {
                tracing::error!(error = %error, "orphan object cleanup failed; retrying");
            }
        }
        health.mark_poll_succeeded(WorkerRole::Cleanup, super::unix_now()?);
        if super::wait_or_shutdown(settings.poll_interval).await {
            return Ok(());
        }
    }
}

#[derive(Default)]
struct OrphanDrainSummary {
    attempted: usize,
    deleted: usize,
    retained: usize,
}

async fn drain_orphan_objects(
    metadata: &MetadataStore,
    object_store: &dyn ObjectStore,
) -> anyhow::Result<OrphanDrainSummary> {
    let cleanup_store = metadata.cleanup();
    let now_unix = super::unix_now()?;
    let batch = cleanup_store
        .source_blob_cleanup_batch(now_unix, &super::generate_persistence_id)
        .await
        .map_err(|error| anyhow::anyhow!("claiming orphan object jobs: {}", error.message))?;
    let mut candidates = BTreeMap::new();
    for object in &batch.pending {
        let object_key = scope_object_store::object_key(object);
        if !batch.referenced_content_refs.contains(&object.content_ref) {
            candidates.entry(object_key).or_insert(object);
        }
    }
    let mut deleted = 0;
    let mut retained = Vec::new();
    for object in candidates.values() {
        match object_store.delete(&scope_object_store::object_key(object)) {
            Ok(()) => deleted += 1,
            Err(error) => {
                tracing::warn!(
                    object_key = %scope_object_store::object_key(object),
                    error = %error.message,
                    "failed to delete orphan object"
                );
                retained.push((*object).clone());
            }
        }
    }
    let summary = OrphanDrainSummary {
        attempted: candidates.len(),
        deleted,
        retained: retained.len(),
    };
    cleanup_store
        .finish_source_blob_cleanup(batch, &retained, now_unix, &super::generate_persistence_id)
        .await
        .map_err(|error| anyhow::anyhow!("finishing orphan object jobs: {}", error.message))?;
    Ok(summary)
}
