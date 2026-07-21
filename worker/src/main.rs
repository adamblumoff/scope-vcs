mod compaction;
mod git_repo;
mod settings;

use crate::{
    compaction::{CompactionOutcome, compact_one_git_repository},
    settings::WorkerSettings,
};
use scope_core::{
    config::{SCOPE_OBJECT_STORE_ENV, non_empty_env},
    db::MetadataStore,
    object_store::{EncryptedObjectStore, FileObjectStore, ObjectStore, S3ObjectStore},
};
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
    time::{Duration, Instant},
};
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

const SCHEMA_WAIT_RETRY_SECS: u64 = 2;
const COMPACTION_RETRY_SECS: u64 = 30;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::registry()
        .with(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "worker=info,scope_core=info".into()),
        )
        .with(tracing_subscriber::fmt::layer())
        .init();

    run().await
}

async fn run() -> anyhow::Result<()> {
    let settings = WorkerSettings::from_env()?;
    tracing::info!(
        worker_id = %settings.worker_id,
        batch_size = settings.batch_size,
        poll_interval_ms = settings.poll_interval.as_millis(),
        schema_wait_secs = settings.schema_wait_timeout.as_secs(),
        git_compaction_segments = settings.git_compaction_segments,
        git_compaction_timeout_secs = settings.git_compaction_timeout.as_secs(),
        git_segment_max_depth = settings.git_storage_limits.max_chain_depth(),
        git_object_max_bytes = settings.git_storage_limits.max_object_bytes(),
        "starting worker"
    );

    let metadata = MetadataStore::connect_worker_from_env_with_schema_wait(
        settings.schema_wait_timeout,
        Duration::from_secs(SCHEMA_WAIT_RETRY_SECS),
    )
    .await?;
    metadata
        .readiness_check()
        .await
        .map_err(|error| anyhow::anyhow!("metadata readiness check failed: {}", error.message))?;
    let object_store = object_store_from_env(&settings.data_dir)?;

    let mut next_compaction_attempt = Instant::now();
    loop {
        let summary = metadata
            .run_ready_outbox_jobs(&settings.worker_id, settings.batch_size)
            .await
            .map_err(|error| anyhow::anyhow!("running outbox jobs: {}", error.message))?;
        if summary.claimed > 0 {
            tracing::info!(
                claimed = summary.claimed,
                completed = summary.completed,
                failed = summary.failed,
                "processed outbox jobs"
            );
        }
        if Instant::now() >= next_compaction_attempt {
            match compact_one_git_repository(
                &metadata,
                object_store.as_ref(),
                settings.git_compaction_segments,
                settings.git_storage_limits,
                settings.git_compaction_timeout,
            )
            .await
            {
                Ok(CompactionOutcome::Applied) => {
                    tracing::info!("compacted Git segment chain")
                }
                Ok(CompactionOutcome::Stale) => {
                    tracing::info!("discarded stale Git compaction result")
                }
                Ok(CompactionOutcome::NoCandidate) => {}
                Ok(CompactionOutcome::Refused(reason)) => {
                    next_compaction_attempt =
                        Instant::now() + Duration::from_secs(COMPACTION_RETRY_SECS);
                    tracing::warn!(
                        reason,
                        retry_seconds = COMPACTION_RETRY_SECS,
                        "Git compaction refused bounded replacement; current head is unchanged"
                    );
                }
                Err(error) => {
                    next_compaction_attempt =
                        Instant::now() + Duration::from_secs(COMPACTION_RETRY_SECS);
                    tracing::error!(
                        error = %error,
                        retry_seconds = COMPACTION_RETRY_SECS,
                        "Git compaction failed; continuing worker loop"
                    );
                }
            }
        }
        let orphan_summary = drain_orphan_objects(&metadata, object_store.as_ref()).await?;
        if orphan_summary.attempted > 0 {
            tracing::info!(
                attempted = orphan_summary.attempted,
                deleted = orphan_summary.deleted,
                retained = orphan_summary.retained,
                "processed orphan object jobs"
            );
        }

        if summary.claimed >= settings.batch_size {
            continue;
        }

        tokio::select! {
            () = shutdown_signal() => {
                tracing::info!("worker shutdown requested");
                return Ok(());
            }
            () = tokio::time::sleep(settings.poll_interval) => {}
        }
    }
}

fn object_store_from_env(data_dir: &std::path::Path) -> anyhow::Result<Arc<dyn ObjectStore>> {
    let raw: Arc<dyn ObjectStore> = match non_empty_env(SCOPE_OBJECT_STORE_ENV).as_deref() {
        Some("filesystem") => Arc::new(FileObjectStore::from_env(&data_dir.join("objects"))),
        Some(value) if value != "s3" => {
            anyhow::bail!("unsupported {SCOPE_OBJECT_STORE_ENV} value {value}")
        }
        _ => Arc::new(S3ObjectStore::from_env()?),
    };
    Ok(Arc::new(EncryptedObjectStore::from_env(raw)?))
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
    let batch = metadata
        .source_blob_cleanup_batch()
        .await
        .map_err(|error| anyhow::anyhow!("claiming orphan object jobs: {}", error.message))?;
    let mut candidates = BTreeMap::new();
    for object in &batch.pending {
        if !batch.referenced_blob_keys.contains(&object.object_key) {
            candidates
                .entry(object.object_key.clone())
                .or_insert(object);
        }
    }
    let mut deleted = BTreeSet::new();
    let mut retained = Vec::new();
    for object in candidates.values() {
        match object_store.delete(&object.object_key) {
            Ok(()) => {
                deleted.insert(object.object_key.clone());
            }
            Err(error) => {
                tracing::warn!(
                    object_key = %object.object_key,
                    error = %error.message,
                    "failed to delete orphan object"
                );
                retained.push((*object).clone());
            }
        }
    }
    let summary = OrphanDrainSummary {
        attempted: candidates.len(),
        deleted: deleted.len(),
        retained: retained.len(),
    };
    metadata
        .finish_source_blob_cleanup(batch, &retained)
        .await
        .map_err(|error| anyhow::anyhow!("finishing orphan object jobs: {}", error.message))?;
    Ok(summary)
}

async fn shutdown_signal() {
    let ctrl_c = async {
        tokio::signal::ctrl_c()
            .await
            .expect("failed to install Ctrl+C handler");
    };

    #[cfg(unix)]
    let terminate = async {
        tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
            .expect("failed to install signal handler")
            .recv()
            .await;
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        () = ctrl_c => {},
        () = terminate => {},
    }
}
