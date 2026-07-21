use scope_core::{
    config::{
        DEFAULT_GIT_COMPACTION_SEGMENTS, SCOPE_DATA_DIR_ENV, git_storage_limits_from_env,
        non_empty_env,
    },
    git_segments::GitStorageLimits,
};
use std::{path::PathBuf, time::Duration};

const DEFAULT_BATCH_SIZE: usize = 10;
const DEFAULT_POLL_INTERVAL_MS: u64 = 1_000;
const DEFAULT_SCHEMA_WAIT_SECS: u64 = 300;
const DEFAULT_GIT_COMPACTION_TIMEOUT_SECS: u64 = 120;

pub(crate) struct WorkerSettings {
    pub(crate) worker_id: String,
    pub(crate) batch_size: usize,
    pub(crate) poll_interval: Duration,
    pub(crate) schema_wait_timeout: Duration,
    pub(crate) git_compaction_segments: usize,
    pub(crate) git_compaction_timeout: Duration,
    pub(crate) git_storage_limits: GitStorageLimits,
    pub(crate) data_dir: PathBuf,
}

impl WorkerSettings {
    pub(crate) fn from_env() -> anyhow::Result<Self> {
        let worker_id = std::env::var("SCOPE_WORKER_ID")
            .ok()
            .filter(|value| !value.trim().is_empty())
            .unwrap_or_else(default_worker_id);
        let batch_size = parse_usize_env("SCOPE_WORKER_BATCH_SIZE", DEFAULT_BATCH_SIZE)?;
        let poll_interval_ms =
            parse_u64_env("SCOPE_WORKER_POLL_INTERVAL_MS", DEFAULT_POLL_INTERVAL_MS)?;
        let schema_wait_secs =
            parse_u64_env("SCOPE_WORKER_SCHEMA_WAIT_SECS", DEFAULT_SCHEMA_WAIT_SECS)?;
        let git_compaction_segments = parse_usize_env(
            "SCOPE_GIT_COMPACTION_SEGMENTS",
            DEFAULT_GIT_COMPACTION_SEGMENTS,
        )?;
        if git_compaction_segments < 2 {
            anyhow::bail!("SCOPE_GIT_COMPACTION_SEGMENTS must be at least 2");
        }
        let git_compaction_timeout_secs = parse_u64_env(
            "SCOPE_GIT_COMPACTION_TIMEOUT_SECS",
            DEFAULT_GIT_COMPACTION_TIMEOUT_SECS,
        )?;
        if git_compaction_timeout_secs == 0 {
            anyhow::bail!("SCOPE_GIT_COMPACTION_TIMEOUT_SECS must be greater than zero");
        }
        let git_storage_limits = git_storage_limits_from_env()?;
        if git_compaction_segments >= git_storage_limits.max_chain_depth() {
            anyhow::bail!(
                "SCOPE_GIT_COMPACTION_SEGMENTS ({git_compaction_segments}) must be lower than SCOPE_GIT_SEGMENT_MAX_DEPTH ({})",
                git_storage_limits.max_chain_depth()
            );
        }
        let data_dir = non_empty_env(SCOPE_DATA_DIR_ENV)
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from(".scope"));
        Ok(Self {
            worker_id,
            batch_size: batch_size.max(1),
            poll_interval: Duration::from_millis(poll_interval_ms.max(100)),
            schema_wait_timeout: Duration::from_secs(schema_wait_secs.max(1)),
            git_compaction_segments,
            git_compaction_timeout: Duration::from_secs(git_compaction_timeout_secs),
            git_storage_limits,
            data_dir,
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
