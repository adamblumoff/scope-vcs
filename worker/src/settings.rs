use scope_git::{
    DEFAULT_GIT_COMPACTION_SPANS, DEFAULT_GIT_STORAGE_MAX_OBJECT_BYTES,
    DEFAULT_GIT_STORAGE_MAX_PACK_SPANS, GitStorageLimits,
};
use std::{path::PathBuf, time::Duration};

const DATABASE_URL_ENV: &str = "DATABASE_URL";
const SCOPE_DATA_DIR_ENV: &str = "SCOPE_DATA_DIR";
const SCOPE_GIT_PACK_SPAN_MAX_COUNT_ENV: &str = "SCOPE_GIT_PACK_SPAN_MAX_COUNT";
const SCOPE_OBJECT_STORE_MAX_BYTES_ENV: &str = "SCOPE_OBJECT_STORE_MAX_BYTES";
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
        let (git_compaction_spans, git_compaction_timeout_secs, git_storage_limits) = if role
            .runs_compaction()
        {
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
            let minimum_span_capacity = spans
                .checked_add(2)
                .ok_or_else(|| anyhow::anyhow!("SCOPE_GIT_COMPACTION_SPANS is too large"))?;
            if limits.max_pack_spans() < minimum_span_capacity {
                anyhow::bail!(
                    "SCOPE_GIT_PACK_SPAN_MAX_COUNT ({}) must be at least two higher than SCOPE_GIT_COMPACTION_SPANS ({spans})",
                    limits.max_pack_spans()
                );
            }
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
            DEFAULT_GIT_STORAGE_MAX_OBJECT_BYTES,
        )?,
        parse_usize_env(
            SCOPE_GIT_PACK_SPAN_MAX_COUNT_ENV,
            DEFAULT_GIT_STORAGE_MAX_PACK_SPANS,
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
}
