use scope_git::GitStorageLimits;
use std::{path::PathBuf, time::Duration};

const DATABASE_URL_ENV: &str = "DATABASE_URL";
const SCOPE_DATA_DIR_ENV: &str = "SCOPE_DATA_DIR";
const SCOPE_GIT_PACK_SPAN_MAX_COUNT_ENV: &str = "SCOPE_GIT_PACK_SPAN_MAX_COUNT";
const SCOPE_OBJECT_STORE_MAX_BYTES_ENV: &str = "SCOPE_OBJECT_STORE_MAX_BYTES";
const DEFAULT_GIT_COMPACTION_SPANS: usize = 32;
const DEFAULT_OBJECT_STORE_MAX_BYTES: usize = 128 * 1024 * 1024;
const DEFAULT_GIT_PACK_SPAN_MAX_COUNT: usize = 2 * DEFAULT_GIT_COMPACTION_SPANS;

const DEFAULT_HEALTH_PORT: u16 = 8081;
const DEFAULT_BATCH_SIZE: usize = 10;
const DEFAULT_POLL_INTERVAL_MS: u64 = 1_000;
const DEFAULT_GIT_COMPACTION_TIMEOUT_SECS: u64 = 120;
const MAX_CLOUD_RUN_CONCURRENCY: usize = 100;

pub(crate) struct WorkerSettings {
    pub(crate) database_url: String,
    pub(crate) health_port: u16,
    pub(crate) worker_id: String,
    pub(crate) batch_size: usize,
    pub(crate) poll_interval: Duration,
    pub(crate) git_compaction_spans: usize,
    pub(crate) git_compaction_timeout: Duration,
    pub(crate) git_storage_limits: GitStorageLimits,
    pub(crate) data_dir: PathBuf,
    pub(crate) execution: Option<CloudExecutionSettings>,
}

#[derive(Clone)]
pub(crate) struct CloudExecutionSettings {
    pub(crate) api_url: String,
    pub(crate) northflank_api_url: String,
    pub(crate) northflank_api_token: String,
    pub(crate) northflank_project_id: String,
    pub(crate) northflank_job_id: String,
    pub(crate) northflank_deployment_plan: String,
    pub(crate) northflank_registry_credentials_id: Option<String>,
    pub(crate) runtime_version: String,
    pub(crate) max_concurrency: usize,
}

impl WorkerSettings {
    pub(crate) fn from_env() -> anyhow::Result<Self> {
        let database_url = required_env(DATABASE_URL_ENV)?;
        let health_port = match non_empty_env("PORT") {
            Some(value) => value
                .parse::<u16>()
                .map_err(|error| anyhow::anyhow!("PORT must be a TCP port: {error}"))?,
            None => DEFAULT_HEALTH_PORT,
        };
        let worker_id = std::env::var("SCOPE_WORKER_ID")
            .ok()
            .filter(|value| !value.trim().is_empty())
            .unwrap_or_else(default_worker_id);
        let batch_size = parse_usize_env("SCOPE_WORKER_BATCH_SIZE", DEFAULT_BATCH_SIZE)?;
        let poll_interval_ms =
            parse_u64_env("SCOPE_WORKER_POLL_INTERVAL_MS", DEFAULT_POLL_INTERVAL_MS)?;
        let git_compaction_spans =
            parse_usize_env("SCOPE_GIT_COMPACTION_SPANS", DEFAULT_GIT_COMPACTION_SPANS)?;
        if git_compaction_spans < 2 {
            anyhow::bail!("SCOPE_GIT_COMPACTION_SPANS must be at least 2");
        }
        let git_compaction_timeout_secs = parse_u64_env(
            "SCOPE_GIT_COMPACTION_TIMEOUT_SECS",
            DEFAULT_GIT_COMPACTION_TIMEOUT_SECS,
        )?;
        if git_compaction_timeout_secs == 0 {
            anyhow::bail!("SCOPE_GIT_COMPACTION_TIMEOUT_SECS must be greater than zero");
        }
        let git_storage_limits = git_storage_limits_from_env()?;
        let minimum_span_capacity = git_compaction_spans
            .checked_add(2)
            .ok_or_else(|| anyhow::anyhow!("SCOPE_GIT_COMPACTION_SPANS is too large"))?;
        if git_storage_limits.max_pack_spans() < minimum_span_capacity {
            anyhow::bail!(
                "SCOPE_GIT_PACK_SPAN_MAX_COUNT ({}) must be at least two higher than SCOPE_GIT_COMPACTION_SPANS ({git_compaction_spans})",
                git_storage_limits.max_pack_spans()
            );
        }
        let data_dir = non_empty_env(SCOPE_DATA_DIR_ENV)
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from(".scope"));
        let execution = cloud_execution_from_env()?;
        Ok(Self {
            database_url,
            health_port,
            worker_id,
            batch_size: batch_size.max(1),
            poll_interval: Duration::from_millis(poll_interval_ms.max(100)),
            git_compaction_spans,
            git_compaction_timeout: Duration::from_secs(git_compaction_timeout_secs),
            git_storage_limits,
            data_dir,
            execution,
        })
    }
}

fn cloud_execution_from_env() -> anyhow::Result<Option<CloudExecutionSettings>> {
    let enabled = non_empty_env("SCOPE_CLOUD_RUNS_ENABLED")
        .is_some_and(|value| matches!(value.as_str(), "1" | "true" | "yes"));
    if !enabled {
        return Ok(None);
    }
    let api_url = required_env("SCOPE_PUBLIC_API_URL")?
        .trim_end_matches('/')
        .to_string();
    if !api_url.starts_with("https://") && !api_url.starts_with("http://127.0.0.1") {
        anyhow::bail!("SCOPE_PUBLIC_API_URL must use HTTPS outside local development");
    }
    let max_concurrency = parse_usize_env("SCOPE_CLOUD_RUNS_MAX_CONCURRENCY", 20)?;
    if !(1..=MAX_CLOUD_RUN_CONCURRENCY).contains(&max_concurrency) {
        anyhow::bail!(
            "SCOPE_CLOUD_RUNS_MAX_CONCURRENCY must be between 1 and {MAX_CLOUD_RUN_CONCURRENCY}"
        );
    }
    Ok(Some(CloudExecutionSettings {
        api_url,
        northflank_api_url: non_empty_env("NORTHFLANK_API_URL")
            .unwrap_or_else(|| "https://api.northflank.com".to_string())
            .trim_end_matches('/')
            .to_string(),
        northflank_api_token: required_env("NORTHFLANK_API_TOKEN")?,
        northflank_project_id: required_env("NORTHFLANK_PROJECT_ID")?,
        northflank_job_id: required_env("NORTHFLANK_JOB_ID")?,
        northflank_deployment_plan: required_env("NORTHFLANK_DEPLOYMENT_PLAN")?,
        northflank_registry_credentials_id: non_empty_env("NORTHFLANK_REGISTRY_CREDENTIALS_ID"),
        runtime_version: non_empty_env("SCOPE_RUNTIME_VERSION")
            .unwrap_or_else(|| env!("CARGO_PKG_VERSION").to_string()),
        max_concurrency,
    }))
}

fn required_env(name: &str) -> anyhow::Result<String> {
    non_empty_env(name).ok_or_else(|| anyhow::anyhow!("{name} is required"))
}

fn non_empty_env(name: &str) -> Option<String> {
    std::env::var(name).ok().filter(|value| !value.is_empty())
}

fn git_storage_limits_from_env() -> anyhow::Result<GitStorageLimits> {
    GitStorageLimits::new(
        parse_usize_env(
            SCOPE_OBJECT_STORE_MAX_BYTES_ENV,
            DEFAULT_OBJECT_STORE_MAX_BYTES,
        )?,
        parse_usize_env(
            SCOPE_GIT_PACK_SPAN_MAX_COUNT_ENV,
            DEFAULT_GIT_PACK_SPAN_MAX_COUNT,
        )?,
    )
    .map_err(anyhow::Error::from)
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
