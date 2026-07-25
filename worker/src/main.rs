mod compaction;
mod git_repo;
mod settings;

use crate::{
    compaction::{CompactionOutcome, compact_one_git_repository},
    settings::WorkerSettings,
};
use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64};
use scope_object_store::{
    EncryptedObjectStore, FileObjectStore, FileObjectStoreSettings, ObjectStore, S3ObjectStore,
    S3ObjectStoreSettings,
};
use scope_postgres::db::{GeneratedIdKind, MetadataStore};
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
    time::{Duration, Instant},
};
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

const SCOPE_BUCKET_ENDPOINT_ENV: &str = "SCOPE_BUCKET_ENDPOINT";
const SCOPE_BUCKET_NAME_ENV: &str = "SCOPE_BUCKET_NAME";
const SCOPE_BUCKET_REGION_ENV: &str = "SCOPE_BUCKET_REGION";
const SCOPE_BUCKET_ACCESS_KEY_ID_ENV: &str = "SCOPE_BUCKET_ACCESS_KEY_ID";
const SCOPE_BUCKET_SECRET_ACCESS_KEY_ENV: &str = "SCOPE_BUCKET_SECRET_ACCESS_KEY";
const SCOPE_BUCKET_FORCE_PATH_STYLE_ENV: &str = "SCOPE_BUCKET_FORCE_PATH_STYLE";
const SCOPE_OBJECT_ENCRYPTION_KEY_ENV: &str = "SCOPE_OBJECT_ENCRYPTION_KEY";
const SCOPE_OBJECT_STORE_ENV: &str = "SCOPE_OBJECT_STORE";
const SCOPE_OBJECT_STORE_DIR_ENV: &str = "SCOPE_OBJECT_STORE_DIR";

const SCHEMA_WAIT_RETRY_SECS: u64 = 2;
const COMPACTION_RETRY_SECS: u64 = 30;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::registry()
        .with(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "worker=info,scope_postgres=info".into()),
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

    let metadata = MetadataStore::connect_worker_with_schema_wait(
        settings.database_url.clone(),
        settings.schema_wait_timeout,
        Duration::from_secs(SCHEMA_WAIT_RETRY_SECS),
    )
    .await?;
    metadata
        .admin()
        .readiness_check()
        .await
        .map_err(|error| anyhow::anyhow!("metadata readiness check failed: {}", error.message))?;
    let object_store = object_store_from_env(&settings.data_dir)?;

    let mut next_compaction_attempt = Instant::now();
    loop {
        let summary = metadata
            .jobs()
            .run_ready_outbox_jobs(&settings.worker_id, settings.batch_size, unix_now()?)
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

fn unix_now() -> anyhow::Result<u64> {
    Ok(std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)?
        .as_secs())
}

fn generate_persistence_id(kind: GeneratedIdKind) -> Result<String, String> {
    let mut bytes = [0_u8; 16];
    getrandom::fill(&mut bytes).map_err(|error| error.to_string())?;
    let random = hex::encode(bytes);
    Ok(match kind {
        GeneratedIdKind::CleanupGeneration => random,
        GeneratedIdKind::OutboxJob => format!("outbox_{random}"),
    })
}

fn object_store_from_env(data_dir: &std::path::Path) -> anyhow::Result<Arc<dyn ObjectStore>> {
    let raw: Arc<dyn ObjectStore> = match non_empty_env(SCOPE_OBJECT_STORE_ENV).as_deref() {
        Some("filesystem") => {
            let root = non_empty_env(SCOPE_OBJECT_STORE_DIR_ENV)
                .map(std::path::PathBuf::from)
                .unwrap_or_else(|| data_dir.join("objects"));
            Arc::new(FileObjectStore::new(FileObjectStoreSettings::new(root)))
        }
        Some(value) if value != "s3" => {
            anyhow::bail!("unsupported {SCOPE_OBJECT_STORE_ENV} value {value}")
        }
        _ => Arc::new(S3ObjectStore::new(s3_settings_from_env()?)?),
    };
    Ok(Arc::new(EncryptedObjectStore::new(
        raw,
        encryption_key_from_env()?,
    )))
}

fn s3_settings_from_env() -> anyhow::Result<S3ObjectStoreSettings> {
    let mut settings = S3ObjectStoreSettings::new(
        required_env(SCOPE_BUCKET_ENDPOINT_ENV)?,
        required_env(SCOPE_BUCKET_NAME_ENV)?,
        required_env(SCOPE_BUCKET_REGION_ENV)?,
        required_env(SCOPE_BUCKET_ACCESS_KEY_ID_ENV)?,
        required_env(SCOPE_BUCKET_SECRET_ACCESS_KEY_ENV)?,
    );
    settings.force_path_style = non_empty_env(SCOPE_BUCKET_FORCE_PATH_STYLE_ENV)
        .map(|value| matches!(value.as_str(), "1" | "true" | "TRUE" | "yes" | "YES"))
        .unwrap_or(false);
    Ok(settings)
}

fn encryption_key_from_env() -> anyhow::Result<[u8; 32]> {
    let encoded = required_env(SCOPE_OBJECT_ENCRYPTION_KEY_ENV)?;
    let decoded = BASE64.decode(encoded.trim()).map_err(|error| {
        anyhow::anyhow!("{SCOPE_OBJECT_ENCRYPTION_KEY_ENV} must be base64: {error}")
    })?;
    decoded.as_slice().try_into().map_err(|_| {
        anyhow::anyhow!("{SCOPE_OBJECT_ENCRYPTION_KEY_ENV} must decode to exactly 32 bytes")
    })
}

fn non_empty_env(name: &str) -> Option<String> {
    std::env::var(name).ok().filter(|value| !value.is_empty())
}

fn required_env(name: &str) -> anyhow::Result<String> {
    non_empty_env(name).ok_or_else(|| anyhow::anyhow!("{name} is required"))
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
    let now_unix = unix_now()?;
    let batch = cleanup_store
        .source_blob_cleanup_batch(now_unix, &generate_persistence_id)
        .await
        .map_err(|error| anyhow::anyhow!("claiming orphan object jobs: {}", error.message))?;
    let mut candidates = BTreeMap::new();
    for object in &batch.pending {
        let object_key = scope_object_store::object_key(object);
        if !batch.referenced_content_refs.contains(&object.content_ref) {
            candidates.entry(object_key).or_insert(object);
        }
    }
    let mut deleted = BTreeSet::new();
    let mut retained = Vec::new();
    for object in candidates.values() {
        match object_store.delete(&scope_object_store::object_key(object)) {
            Ok(()) => {
                deleted.insert(scope_object_store::object_key(object));
            }
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
        deleted: deleted.len(),
        retained: retained.len(),
    };
    cleanup_store
        .finish_source_blob_cleanup(batch, &retained, now_unix, &generate_persistence_id)
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
