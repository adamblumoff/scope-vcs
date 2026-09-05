mod cleanup;
mod compaction;
mod control;
mod execution;
mod git_repo;
mod health;
mod run_events;
mod settings;

use crate::{
    health::WorkerHealth,
    settings::{WorkerRole, WorkerSettings},
};
use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64};
use scope_git_storage::{
    FileMultipartStore, GitSegmentStore, MultipartStore, S3MultipartSettings, S3MultipartStore,
    SegmentEncryptionKey,
};
use scope_object_store::{
    EncryptedObjectStore, FileObjectStore, FileObjectStoreSettings, ObjectStore, S3ObjectStore,
    S3ObjectStoreSettings,
};
use scope_postgres::db::{GeneratedIdKind, MetadataStore};
use std::{process::Command, sync::Arc, time::Duration};
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
        role = settings.role.as_str(),
        health_port = settings.health_port,
        batch_size = settings.batch_size,
        poll_interval_ms = settings.poll_interval.as_millis(),
        git_compaction_spans = settings.git_compaction_spans,
        git_compaction_timeout_secs = settings.git_compaction_timeout.as_secs(),
        git_object_max_bytes = settings.git_storage_limits.max_object_bytes(),
        git_segment_chunk_bytes = settings.git_segment_store.chunk_bytes,
        git_segment_multipart_part_bytes = settings.git_segment_store.multipart_part_bytes,
        git_segment_channel_capacity = settings.git_segment_store.channel_capacity,
        "starting worker"
    );
    if let Some(execution) = settings.execution.as_ref() {
        tracing::info!(
            ecs_capacity_provider = execution.ecs_capacity.capacity_provider(),
            ecs_task_memory_mib = execution.ecs_task_memory_mib,
            ecs_task_family_prefix = %execution.ecs_task_family_prefix,
            cloud_run_max_concurrency = execution.max_concurrency,
            "configured cloud execution"
        );
    }

    let health = WorkerHealth::new(settings.poll_interval, settings.role);
    let health_server = health.clone().serve(settings.health_port);
    let worker = run_worker(settings, health);
    tokio::try_join!(health_server, worker)?;
    Ok(())
}

async fn run_worker(settings: WorkerSettings, health: WorkerHealth) -> anyhow::Result<()> {
    if settings.role.runs_compaction() {
        require_git_runtime()?;
    }
    let Some(metadata) = connect_worker_or_wait(&settings, &health).await else {
        return Ok(());
    };
    let object_store = if settings.role.runs_cleanup() {
        Some(object_store_from_env(&settings.data_dir)?)
    } else {
        None
    };
    let git_segment_store = if settings.role.runs_compaction() {
        Some(Arc::new(git_segment_store_from_env(&settings)?))
    } else {
        None
    };
    match settings.role {
        WorkerRole::All => {
            let object_store = object_store.expect("all roles require object storage");
            let git_segment_store =
                git_segment_store.expect("all roles require Git segment storage");
            tokio::try_join!(
                control::run(metadata.clone(), settings.clone(), health.clone()),
                compaction::run(
                    metadata.clone(),
                    git_segment_store,
                    settings.clone(),
                    health.clone(),
                ),
                cleanup::run(metadata, object_store, settings, health),
            )?;
        }
        WorkerRole::Control => control::run(metadata, settings, health).await?,
        WorkerRole::Compaction => {
            compaction::run(
                metadata,
                git_segment_store.expect("compaction role requires Git segment storage"),
                settings,
                health,
            )
            .await?;
        }
        WorkerRole::Cleanup => {
            cleanup::run(
                metadata,
                object_store.expect("cleanup role requires object storage"),
                settings,
                health,
            )
            .await?;
        }
    }
    Ok(())
}

fn require_git_runtime() -> anyhow::Result<()> {
    let output = Command::new("git")
        .arg("--version")
        .output()
        .map_err(|error| anyhow::anyhow!("Git compaction requires the git executable: {error}"))?;
    if !output.status.success() {
        anyhow::bail!(
            "Git compaction requires a working git executable: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    Ok(())
}

async fn connect_worker_or_wait(
    settings: &WorkerSettings,
    health: &WorkerHealth,
) -> Option<MetadataStore> {
    loop {
        match MetadataStore::connect_worker(settings.database_url.clone()).await {
            Ok(metadata) => return Some(metadata),
            Err(error) => {
                health.mark_schema_waiting();
                tracing::warn!(
                    error = %error,
                    retry_in_secs = SCHEMA_WAIT_RETRY_SECS,
                    "metadata is unavailable or behind; waiting before worker startup"
                );
                if wait_or_shutdown(Duration::from_secs(SCHEMA_WAIT_RETRY_SECS)).await {
                    return None;
                }
            }
        }
    }
}

async fn schema_ready_or_wait(metadata: &MetadataStore, health: &WorkerHealth) -> bool {
    loop {
        match metadata.admin().readiness_check().await {
            Ok(()) => return true,
            Err(error) => {
                health.mark_schema_waiting();
                tracing::warn!(
                    error = %error.message,
                    retry_in_secs = SCHEMA_WAIT_RETRY_SECS,
                    "metadata migration state changed; pausing worker role"
                );
                if wait_or_shutdown(Duration::from_secs(SCHEMA_WAIT_RETRY_SECS)).await {
                    return false;
                }
            }
        }
    }
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
        GeneratedIdKind::RepositoryIncarnation => format!("repoi_{random}"),
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

fn git_segment_store_from_env(settings: &WorkerSettings) -> anyhow::Result<GitSegmentStore> {
    let backend: Arc<dyn MultipartStore> = match non_empty_env(SCOPE_OBJECT_STORE_ENV).as_deref() {
        Some("filesystem") => {
            let root = non_empty_env(SCOPE_OBJECT_STORE_DIR_ENV)
                .map(std::path::PathBuf::from)
                .unwrap_or_else(|| settings.data_dir.join("objects"));
            Arc::new(FileMultipartStore::new(root)?)
        }
        Some(value) if value != "s3" => {
            anyhow::bail!("unsupported {SCOPE_OBJECT_STORE_ENV} value {value}")
        }
        _ => {
            let s3 = s3_settings_from_env()?;
            Arc::new(S3MultipartStore::new(S3MultipartSettings {
                endpoint: s3.endpoint,
                bucket: s3.bucket,
                region: s3.region,
                access_key_id: s3.access_key_id,
                secret_access_key: s3.secret_access_key,
                force_path_style: s3.force_path_style,
            })?)
        }
    };
    GitSegmentStore::new(
        backend,
        SegmentEncryptionKey::new("primary", encryption_key_from_env()?)?,
        settings.git_segment_store.clone(),
    )
    .map_err(anyhow::Error::from)
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
