use scope_core::db::MetadataStore;
use std::time::Duration;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

const DEFAULT_BATCH_SIZE: usize = 10;
const DEFAULT_POLL_INTERVAL_MS: u64 = 1_000;
const DEFAULT_SCHEMA_WAIT_SECS: u64 = 300;
const SCHEMA_WAIT_RETRY_SECS: u64 = 2;

fn main() -> anyhow::Result<()> {
    tracing_subscriber::registry()
        .with(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "worker=info,scope_core=info".into()),
        )
        .with(tracing_subscriber::fmt::layer())
        .init();

    tokio::runtime::Runtime::new()?.block_on(run())
}

async fn run() -> anyhow::Result<()> {
    let settings = WorkerSettings::from_env()?;
    tracing::info!(
        worker_id = %settings.worker_id,
        batch_size = settings.batch_size,
        poll_interval_ms = settings.poll_interval.as_millis(),
        schema_wait_secs = settings.schema_wait_timeout.as_secs(),
        "starting worker"
    );

    let metadata = MetadataStore::connect_worker_from_env_with_schema_wait(
        settings.schema_wait_timeout,
        Duration::from_secs(SCHEMA_WAIT_RETRY_SECS),
    )?;
    metadata
        .readiness_check()
        .map_err(|error| anyhow::anyhow!("metadata readiness check failed: {}", error.message))?;

    loop {
        let summary = metadata
            .run_ready_outbox_jobs(&settings.worker_id, settings.batch_size)
            .map_err(|error| anyhow::anyhow!("running outbox jobs: {}", error.message))?;
        if summary.claimed > 0 {
            tracing::info!(
                claimed = summary.claimed,
                completed = summary.completed,
                failed = summary.failed,
                "processed outbox jobs"
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

struct WorkerSettings {
    worker_id: String,
    batch_size: usize,
    poll_interval: Duration,
    schema_wait_timeout: Duration,
}

impl WorkerSettings {
    fn from_env() -> anyhow::Result<Self> {
        let worker_id = std::env::var("SCOPE_WORKER_ID")
            .ok()
            .filter(|value| !value.trim().is_empty())
            .unwrap_or_else(default_worker_id);
        let batch_size = parse_usize_env("SCOPE_WORKER_BATCH_SIZE", DEFAULT_BATCH_SIZE)?;
        let poll_interval_ms =
            parse_u64_env("SCOPE_WORKER_POLL_INTERVAL_MS", DEFAULT_POLL_INTERVAL_MS)?;
        let schema_wait_secs =
            parse_u64_env("SCOPE_WORKER_SCHEMA_WAIT_SECS", DEFAULT_SCHEMA_WAIT_SECS)?;
        Ok(Self {
            worker_id,
            batch_size: batch_size.max(1),
            poll_interval: Duration::from_millis(poll_interval_ms.max(100)),
            schema_wait_timeout: Duration::from_secs(schema_wait_secs.max(1)),
        })
    }
}

fn parse_usize_env(name: &str, default: usize) -> anyhow::Result<usize> {
    match std::env::var(name) {
        Ok(value) if !value.trim().is_empty() => value
            .parse::<usize>()
            .map_err(|error| anyhow::anyhow!("{name} must be an integer: {error}")),
        _ => Ok(default),
    }
}

fn parse_u64_env(name: &str, default: u64) -> anyhow::Result<u64> {
    match std::env::var(name) {
        Ok(value) if !value.trim().is_empty() => value
            .parse::<u64>()
            .map_err(|error| anyhow::anyhow!("{name} must be an integer: {error}")),
        _ => Ok(default),
    }
}

fn default_worker_id() -> String {
    let host = std::env::var("RAILWAY_REPLICA_ID")
        .or_else(|_| std::env::var("HOSTNAME"))
        .unwrap_or_else(|_| "local".to_string());
    format!("scope-worker-{host}-{}", std::process::id())
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
