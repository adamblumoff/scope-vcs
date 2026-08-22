use crate::{
    health::WorkerHealth,
    settings::{WorkerRole, WorkerSettings},
};
use scope_object_store::ObjectStore;
use scope_postgres::db::MetadataStore;
use std::sync::Arc;

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
    let now_unix = super::unix_now()?;
    let report = scope_content_lifecycle::drain_source_blob_cleanup(
        metadata,
        object_store,
        now_unix,
        &super::generate_persistence_id,
    )
    .await
    .map_err(|error| anyhow::anyhow!("draining orphan object jobs: {}", error.message))?;
    for failure in &report.failed_object_deletes {
        tracing::warn!(
            object_key = %scope_object_store::object_key(&failure.blob),
            error = %failure.error.message,
            "failed to delete orphan object"
        );
    }
    let summary = OrphanDrainSummary {
        attempted: report.attempted,
        deleted: report.deleted,
        retained: report.retained,
    };
    Ok(summary)
}
