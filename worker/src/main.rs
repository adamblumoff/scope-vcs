mod compaction;
mod execution;
mod git_repo;
mod health;
mod settings;

use crate::{
    compaction::{CompactionOutcome, compact_one_git_repository},
    execution::CloudExecutionCoordinator,
    health::WorkerHealth,
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
const RUNTIME_TELEMETRY_INTERVAL_SECS_ENV: &str = "SCOPE_RUNTIME_TELEMETRY_INTERVAL_SECS";

const SCHEMA_WAIT_RETRY_SECS: u64 = 2;
const COMPACTION_RETRY_SECS: u64 = 30;
const CLEANUP_RETRY_SECS: u64 = 30;

fn main() -> anyhow::Result<()> {
    scope_git_process::install_pid1_reaper_if_needed()?;
    run_service()
}

#[tokio::main]
async fn run_service() -> anyhow::Result<()> {
    tracing_subscriber::registry()
        .with(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "worker=info,scope_postgres=info".into()),
        )
        .with(tracing_subscriber::fmt::layer())
        .init();

    start_runtime_telemetry();

    run().await
}

fn start_runtime_telemetry() {
    let Some(interval) = std::env::var(RUNTIME_TELEMETRY_INTERVAL_SECS_ENV)
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
        .filter(|seconds| *seconds > 0)
        .map(Duration::from_secs)
    else {
        return;
    };
    tokio::spawn(async move {
        loop {
            let snapshot = scope_git_process::current_process_snapshot();
            tracing::info!(
                process_id = snapshot.process_id,
                parent_process_id = snapshot.parent_process_id.unwrap_or(0),
                threads = snapshot.threads.unwrap_or(0),
                open_file_descriptors = snapshot.open_file_descriptors.unwrap_or(0),
                child_processes = snapshot.child_processes.unwrap_or(0),
                zombie_child_processes = snapshot.zombie_child_processes.unwrap_or(0),
                cgroup_pids_current = snapshot.cgroup_pids_current.unwrap_or(0),
                cgroup_pids_max = snapshot.cgroup_pids_max.unwrap_or(0),
                cgroup_pids_unlimited = snapshot.cgroup_pids_unlimited,
                "runtime process snapshot"
            );
            tokio::time::sleep(interval).await;
        }
    });
}

async fn run() -> anyhow::Result<()> {
    let settings = WorkerSettings::from_env()?;
    tracing::info!(
        worker_id = %settings.worker_id,
        health_port = settings.health_port,
        batch_size = settings.batch_size,
        poll_interval_ms = settings.poll_interval.as_millis(),
        git_compaction_spans = settings.git_compaction_spans,
        git_compaction_timeout_secs = settings.git_compaction_timeout.as_secs(),
        git_pack_span_max_count = settings.git_storage_limits.max_pack_spans(),
        git_object_max_bytes = settings.git_storage_limits.max_object_bytes(),
        "starting worker"
    );

    let health = WorkerHealth::new(settings.poll_interval);
    let health_server = health.clone().serve(settings.health_port);
    let worker = run_worker(settings, health);
    tokio::try_join!(health_server, worker)?;
    Ok(())
}

async fn run_worker(settings: WorkerSettings, health: WorkerHealth) -> anyhow::Result<()> {
    let metadata = MetadataStore::connect_worker(settings.database_url.clone()).await?;
    let object_store = object_store_from_env(&settings.data_dir)?;
    let execution = settings
        .execution
        .clone()
        .map(|cloud| CloudExecutionCoordinator::new(metadata.clone(), cloud))
        .transpose()?;

    let mut next_compaction_attempt = Instant::now();
    let mut next_cleanup_attempt = Instant::now();
    loop {
        let mut compaction_made_progress = false;
        if let Err(error) = metadata.admin().readiness_check().await {
            health.mark_schema_waiting();
            tracing::warn!(
                error = %error.message,
                retry_in_secs = SCHEMA_WAIT_RETRY_SECS,
                "metadata migration state changed; pausing worker"
            );
            if wait_or_shutdown(Duration::from_secs(SCHEMA_WAIT_RETRY_SECS)).await {
                return Ok(());
            }
            continue;
        }

        let summary = match metadata
            .jobs()
            .run_ready_outbox_jobs(
                &settings.worker_id,
                settings.batch_size,
                &|| unix_now().map_err(|error| error.to_string()),
                &generate_persistence_id,
            )
            .await
        {
            Ok(summary) => summary,
            Err(error) => {
                health.mark_schema_waiting();
                tracing::error!(
                    error = %error.message,
                    retry_in_secs = SCHEMA_WAIT_RETRY_SECS,
                    "outbox polling failed; pausing worker"
                );
                if wait_or_shutdown(Duration::from_secs(SCHEMA_WAIT_RETRY_SECS)).await {
                    return Ok(());
                }
                continue;
            }
        };
        if summary.claimed > 0 {
            tracing::info!(
                claimed = summary.claimed,
                completed = summary.completed,
                failed = summary.failed,
                "processed outbox jobs"
            );
        }
        if let Some(execution) = &execution {
            if let Err(error) = execution.abort_canceled(unix_now()?).await {
                tracing::error!(error = %error, "cloud run cancellation reconciliation failed");
            }
            match execution.dispatch_available(unix_now()?).await {
                Ok(dispatched) if dispatched > 0 => {
                    tracing::info!(dispatched, "processed cloud run dispatches");
                }
                Ok(_) => {}
                Err(error) => {
                    tracing::error!(error = %error, "cloud run dispatch failed; continuing worker loop");
                }
            }
        }
        if Instant::now() >= next_compaction_attempt {
            match compact_one_git_repository(
                &metadata,
                object_store.as_ref(),
                settings.git_compaction_spans,
                settings.git_storage_limits,
                settings.git_compaction_timeout,
            )
            .await
            {
                Ok(CompactionOutcome::Applied | CompactionOutcome::Stale) => {
                    compaction_made_progress = true;
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
        if Instant::now() >= next_cleanup_attempt {
            match drain_orphan_objects(&metadata, object_store.as_ref()).await {
                Ok(orphan_summary) => {
                    if orphan_summary.attempted > 0 {
                        tracing::info!(
                            attempted = orphan_summary.attempted,
                            deleted = orphan_summary.deleted,
                            retained = orphan_summary.retained,
                            "processed orphan object jobs"
                        );
                    }
                }
                Err(error) => {
                    next_cleanup_attempt = Instant::now() + Duration::from_secs(CLEANUP_RETRY_SECS);
                    tracing::error!(
                        error = %error,
                        retry_seconds = CLEANUP_RETRY_SECS,
                        "orphan object cleanup failed; continuing projection processing"
                    );
                }
            }
        }
        health.mark_poll_succeeded(unix_now()?);

        if should_poll_immediately(
            summary.claimed,
            settings.batch_size,
            compaction_made_progress,
        ) {
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

fn should_poll_immediately(
    claimed_outbox_jobs: usize,
    batch_size: usize,
    compaction_made_progress: bool,
) -> bool {
    claimed_outbox_jobs >= batch_size || compaction_made_progress
}

async fn wait_or_shutdown(duration: Duration) -> bool {
    tokio::select! {
        () = shutdown_signal() => true,
        () = tokio::time::sleep(duration) => false,
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

#[cfg(test)]
mod tests {
    use super::should_poll_immediately;

    #[test]
    fn useful_compaction_drains_without_waiting_for_the_poll_interval() {
        assert!(should_poll_immediately(0, 10, true));
        assert!(should_poll_immediately(10, 10, false));
        assert!(!should_poll_immediately(0, 10, false));
    }
}
