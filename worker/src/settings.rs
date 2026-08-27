use scope_git::{
    DEFAULT_GIT_COMPACTION_SPANS, DEFAULT_GIT_STORAGE_MAX_OBJECT_BYTES, GitStorageLimits,
};
use scope_git_storage::GitSegmentStoreConfig;
use std::{path::PathBuf, time::Duration};

const DATABASE_URL_ENV: &str = "DATABASE_URL";
const SCOPE_DATA_DIR_ENV: &str = "SCOPE_DATA_DIR";
const SCOPE_OBJECT_STORE_MAX_BYTES_ENV: &str = "SCOPE_OBJECT_STORE_MAX_BYTES";
const SCOPE_GIT_SEGMENT_CHUNK_BYTES_ENV: &str = "SCOPE_GIT_SEGMENT_CHUNK_BYTES";
const SCOPE_GIT_SEGMENT_MULTIPART_PART_BYTES_ENV: &str = "SCOPE_GIT_SEGMENT_MULTIPART_PART_BYTES";
const SCOPE_GIT_SEGMENT_CHANNEL_CAPACITY_ENV: &str = "SCOPE_GIT_SEGMENT_CHANNEL_CAPACITY";
const DEFAULT_HEALTH_PORT: u16 = 8081;
const DEFAULT_BATCH_SIZE: usize = 10;
const DEFAULT_POLL_INTERVAL_MS: u64 = 1_000;
const DEFAULT_GIT_COMPACTION_TIMEOUT_SECS: u64 = 120;
const MAX_CLOUD_RUN_CONCURRENCY: usize = 100;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum WorkerRole {
    All,
    Control,
    Compaction,
    Cleanup,
}

impl WorkerRole {
    pub(crate) fn runs_control(self) -> bool {
        matches!(self, Self::All | Self::Control)
    }

    pub(crate) fn runs_compaction(self) -> bool {
        matches!(self, Self::All | Self::Compaction)
    }

    pub(crate) fn runs_cleanup(self) -> bool {
        matches!(self, Self::All | Self::Cleanup)
    }

    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::All => "all",
            Self::Control => "control",
            Self::Compaction => "compaction",
            Self::Cleanup => "cleanup",
        }
    }
}

#[derive(Clone)]
pub(crate) struct WorkerSettings {
    pub(crate) role: WorkerRole,
    pub(crate) database_url: String,
    pub(crate) health_port: u16,
    pub(crate) worker_id: String,
    pub(crate) batch_size: usize,
    pub(crate) poll_interval: Duration,
    pub(crate) git_compaction_spans: usize,
    pub(crate) git_compaction_timeout: Duration,
    pub(crate) git_storage_limits: GitStorageLimits,
    pub(crate) git_segment_store: GitSegmentStoreConfig,
    pub(crate) data_dir: PathBuf,
    pub(crate) execution: Option<CloudExecutionSettings>,
}

#[derive(Clone)]
pub(crate) struct CloudExecutionSettings {
    pub(crate) api_url: String,
    pub(crate) aws_region: String,
    pub(crate) ecs_cluster_arn: String,
    pub(crate) ecs_subnet_ids: Vec<String>,
    pub(crate) ecs_security_group_id: String,
    pub(crate) ecs_execution_role_arn: String,
    pub(crate) ecs_log_group: String,
    pub(crate) ecs_secret_name_key: [u8; 32],
    pub(crate) runtime_version: String,
    pub(crate) max_concurrency: usize,
}

impl WorkerSettings {
    pub(crate) fn from_env() -> anyhow::Result<Self> {
        let database_url = required_env(DATABASE_URL_ENV)?;
        let role = worker_role_from_env()?;
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
        let batch_size = if role.runs_control() {
            parse_usize_env("SCOPE_WORKER_BATCH_SIZE", DEFAULT_BATCH_SIZE)?
        } else {
            DEFAULT_BATCH_SIZE
        };
        let poll_interval_ms =
            parse_u64_env("SCOPE_WORKER_POLL_INTERVAL_MS", DEFAULT_POLL_INTERVAL_MS)?;
        let (git_compaction_spans, git_compaction_timeout_secs, git_storage_limits) =
            if role.runs_compaction() {
                let spans =
                    parse_usize_env("SCOPE_GIT_COMPACTION_SPANS", DEFAULT_GIT_COMPACTION_SPANS)?;
                if spans < 2 {
                    anyhow::bail!("SCOPE_GIT_COMPACTION_SPANS must be at least 2");
                }
                let timeout_secs = parse_u64_env(
                    "SCOPE_GIT_COMPACTION_TIMEOUT_SECS",
                    DEFAULT_GIT_COMPACTION_TIMEOUT_SECS,
                )?;
                if timeout_secs == 0 {
                    anyhow::bail!("SCOPE_GIT_COMPACTION_TIMEOUT_SECS must be greater than zero");
                }
                let limits = git_storage_limits_from_env()?;
                (spans, timeout_secs, limits)
            } else {
                (
                    DEFAULT_GIT_COMPACTION_SPANS,
                    DEFAULT_GIT_COMPACTION_TIMEOUT_SECS,
                    GitStorageLimits::default(),
                )
            };
        let data_dir = non_empty_env(SCOPE_DATA_DIR_ENV)
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from(".scope"));
        let mut git_segment_store = GitSegmentStoreConfig::new(data_dir.join("git-segments"));
        if role.runs_compaction() {
            git_segment_store.chunk_bytes = parse_usize_env(
                SCOPE_GIT_SEGMENT_CHUNK_BYTES_ENV,
                git_segment_store.chunk_bytes,
            )?;
            git_segment_store.multipart_part_bytes = parse_usize_env(
                SCOPE_GIT_SEGMENT_MULTIPART_PART_BYTES_ENV,
                git_segment_store.multipart_part_bytes,
            )?;
            git_segment_store.channel_capacity = parse_usize_env(
                SCOPE_GIT_SEGMENT_CHANNEL_CAPACITY_ENV,
                git_segment_store.channel_capacity,
            )?;
        }
        let execution = if role.runs_control() {
            cloud_execution_from_env()?
        } else {
            None
        };
        Ok(Self {
            role,
            database_url,
            health_port,
            worker_id,
            batch_size: batch_size.max(1),
            poll_interval: Duration::from_millis(poll_interval_ms.max(100)),
            git_compaction_spans,
            git_compaction_timeout: Duration::from_secs(git_compaction_timeout_secs),
            git_storage_limits,
            git_segment_store,
            data_dir,
            execution,
        })
    }
}

fn worker_role_from_env() -> anyhow::Result<WorkerRole> {
    parse_worker_role(non_empty_env("SCOPE_WORKER_ROLE").as_deref())
}

fn parse_worker_role(value: Option<&str>) -> anyhow::Result<WorkerRole> {
    match value {
        None | Some("all") => Ok(WorkerRole::All),
        Some("control") => Ok(WorkerRole::Control),
        Some("compaction") => Ok(WorkerRole::Compaction),
        Some("cleanup") => Ok(WorkerRole::Cleanup),
        Some(value) => anyhow::bail!(
            "SCOPE_WORKER_ROLE must be all, control, compaction, or cleanup; found {value}"
        ),
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
        aws_region: required_env("AWS_REGION")?,
        ecs_cluster_arn: required_env("SCOPE_ECS_CLUSTER_ARN")?,
        ecs_subnet_ids: comma_separated_env("SCOPE_ECS_SUBNET_IDS")?,
        ecs_security_group_id: required_env("SCOPE_ECS_SECURITY_GROUP_ID")?,
        ecs_execution_role_arn: required_env("SCOPE_ECS_EXECUTION_ROLE_ARN")?,
        ecs_log_group: required_env("SCOPE_ECS_LOG_GROUP")?,
        ecs_secret_name_key: secret_name_key_from_env()?,
        runtime_version: non_empty_env("SCOPE_RUNTIME_VERSION")
            .unwrap_or_else(|| env!("CARGO_PKG_VERSION").to_string()),
        max_concurrency,
    }))
}

fn secret_name_key_from_env() -> anyhow::Result<[u8; 32]> {
    let encoded = required_env("SCOPE_ECS_SECRET_NAME_KEY")?;
    parse_secret_name_key(&encoded)
}

fn parse_secret_name_key(encoded: &str) -> anyhow::Result<[u8; 32]> {
    let decoded = hex::decode(encoded).map_err(|_| {
        anyhow::anyhow!("SCOPE_ECS_SECRET_NAME_KEY must be 64 hexadecimal characters")
    })?;
    decoded
        .try_into()
        .map_err(|_| anyhow::anyhow!("SCOPE_ECS_SECRET_NAME_KEY must be 64 hexadecimal characters"))
}

fn comma_separated_env(name: &str) -> anyhow::Result<Vec<String>> {
    parse_comma_separated(name, &required_env(name)?)
}

fn parse_comma_separated(name: &str, value: &str) -> anyhow::Result<Vec<String>> {
    let values = value
        .split(',')
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .collect::<Vec<_>>();
    if values.is_empty() {
        anyhow::bail!("{name} must contain at least one value");
    }
    Ok(values)
}

fn required_env(name: &str) -> anyhow::Result<String> {
    non_empty_env(name).ok_or_else(|| anyhow::anyhow!("{name} is required"))
}

fn non_empty_env(name: &str) -> Option<String> {
    std::env::var(name).ok().filter(|value| !value.is_empty())
}

fn git_storage_limits_from_env() -> anyhow::Result<GitStorageLimits> {
    GitStorageLimits::new(parse_usize_env(
        SCOPE_OBJECT_STORE_MAX_BYTES_ENV,
        DEFAULT_GIT_STORAGE_MAX_OBJECT_BYTES,
    )?)
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn worker_roles_have_one_canonical_spelling() {
        assert_eq!(parse_worker_role(None).unwrap(), WorkerRole::All);
        assert_eq!(parse_worker_role(Some("all")).unwrap(), WorkerRole::All);
        assert_eq!(
            parse_worker_role(Some("control")).unwrap(),
            WorkerRole::Control
        );
        assert_eq!(
            parse_worker_role(Some("compaction")).unwrap(),
            WorkerRole::Compaction
        );
        assert_eq!(
            parse_worker_role(Some("cleanup")).unwrap(),
            WorkerRole::Cleanup
        );
        assert!(parse_worker_role(Some("worker")).is_err());
    }

    #[test]
    fn comma_separated_settings_ignore_only_empty_segments() {
        assert_eq!(
            parse_comma_separated("SUBNETS", " subnet-a,subnet-b ,, ").unwrap(),
            ["subnet-a", "subnet-b"]
        );
        assert!(parse_comma_separated("SUBNETS", " , ").is_err());
    }

    #[test]
    fn secret_name_key_requires_exactly_32_hex_encoded_bytes() {
        assert_eq!(parse_secret_name_key(&"ab".repeat(32)).unwrap(), [0xab; 32]);
        assert!(parse_secret_name_key(&"ab".repeat(31)).is_err());
        assert!(parse_secret_name_key(&"xy".repeat(32)).is_err());
    }
}
