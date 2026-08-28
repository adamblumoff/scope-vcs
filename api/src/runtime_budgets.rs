use crate::{
    config::{default_git_storage_limits, git_storage_limits_from_env},
    error::ApiError,
};
use scope_git::GitStorageLimits;
use scope_object_store::{ObjectStore, ObjectStoreError, ensure_object_size};
use std::{
    sync::{Arc, OnceLock},
    time::{Duration, Instant},
};
use tokio::sync::{OwnedSemaphorePermit, Semaphore};

const DEFAULT_RECEIVE_PACK_CONCURRENCY: usize = 4;
const DEFAULT_UPLOAD_PACK_CONCURRENCY: usize = 8;
const DEFAULT_GIT_MATERIALIZATION_CONCURRENCY: usize = 2;
const DEFAULT_GIT_SEGMENT_INGEST_CONCURRENCY: usize = 4;
const DEFAULT_OBJECT_STORE_CONCURRENCY: usize = 16;
const DEFAULT_GIT_COMMAND_TIMEOUT_SECS: u64 = 30;
const DEFAULT_GIT_MATERIALIZATION_WAIT_MILLIS: u64 = 1_000;
const MAX_GIT_MATERIALIZATION_WAIT_MILLIS: u64 = 5_000;

const RECEIVE_PACK_CONCURRENCY_ENV: &str = "SCOPE_GIT_RECEIVE_PACK_CONCURRENCY";
const UPLOAD_PACK_CONCURRENCY_ENV: &str = "SCOPE_GIT_UPLOAD_PACK_CONCURRENCY";
const GIT_MATERIALIZATION_CONCURRENCY_ENV: &str = "SCOPE_GIT_MATERIALIZATION_CONCURRENCY";
const GIT_SEGMENT_INGEST_CONCURRENCY_ENV: &str = "SCOPE_GIT_SEGMENT_INGEST_CONCURRENCY";
const OBJECT_STORE_CONCURRENCY_ENV: &str = "SCOPE_OBJECT_STORE_CONCURRENCY";
const GIT_COMMAND_TIMEOUT_SECS_ENV: &str = "SCOPE_GIT_COMMAND_TIMEOUT_SECS";
const GIT_MATERIALIZATION_WAIT_MILLIS_ENV: &str = "SCOPE_GIT_MATERIALIZATION_WAIT_MILLIS";

#[derive(Clone, Debug)]
pub(crate) struct RuntimeBudgetConfig {
    pub(crate) receive_pack_concurrency: usize,
    pub(crate) upload_pack_concurrency: usize,
    pub(crate) git_materialization_concurrency: usize,
    pub(crate) git_segment_ingest_concurrency: usize,
    pub(crate) object_store_concurrency: usize,
    pub(crate) git_materialization_wait: Duration,
    pub(crate) git_command_timeout: Duration,
    pub(crate) git_storage_limits: GitStorageLimits,
}

impl RuntimeBudgetConfig {
    pub(crate) fn from_env() -> anyhow::Result<Self> {
        Ok(Self {
            receive_pack_concurrency: parse_usize_env(
                RECEIVE_PACK_CONCURRENCY_ENV,
                DEFAULT_RECEIVE_PACK_CONCURRENCY,
            )?,
            upload_pack_concurrency: parse_usize_env(
                UPLOAD_PACK_CONCURRENCY_ENV,
                DEFAULT_UPLOAD_PACK_CONCURRENCY,
            )?,
            git_materialization_concurrency: parse_usize_env(
                GIT_MATERIALIZATION_CONCURRENCY_ENV,
                DEFAULT_GIT_MATERIALIZATION_CONCURRENCY,
            )?,
            git_segment_ingest_concurrency: parse_usize_env(
                GIT_SEGMENT_INGEST_CONCURRENCY_ENV,
                DEFAULT_GIT_SEGMENT_INGEST_CONCURRENCY,
            )?,
            object_store_concurrency: parse_usize_env(
                OBJECT_STORE_CONCURRENCY_ENV,
                DEFAULT_OBJECT_STORE_CONCURRENCY,
            )?,
            git_materialization_wait: git_materialization_wait_from_env()?,
            git_command_timeout: Duration::from_secs(parse_u64_env(
                GIT_COMMAND_TIMEOUT_SECS_ENV,
                DEFAULT_GIT_COMMAND_TIMEOUT_SECS,
            )?),
            git_storage_limits: git_storage_limits_from_env()?,
        })
    }
}

impl Default for RuntimeBudgetConfig {
    fn default() -> Self {
        Self {
            receive_pack_concurrency: DEFAULT_RECEIVE_PACK_CONCURRENCY,
            upload_pack_concurrency: DEFAULT_UPLOAD_PACK_CONCURRENCY,
            git_materialization_concurrency: DEFAULT_GIT_MATERIALIZATION_CONCURRENCY,
            git_segment_ingest_concurrency: DEFAULT_GIT_SEGMENT_INGEST_CONCURRENCY,
            object_store_concurrency: DEFAULT_OBJECT_STORE_CONCURRENCY,
            git_materialization_wait: Duration::from_millis(
                DEFAULT_GIT_MATERIALIZATION_WAIT_MILLIS,
            ),
            git_command_timeout: Duration::from_secs(DEFAULT_GIT_COMMAND_TIMEOUT_SECS),
            git_storage_limits: default_git_storage_limits(),
        }
    }
}

pub(crate) struct RuntimeBudgets {
    receive_pack: Arc<Semaphore>,
    upload_pack: Arc<Semaphore>,
    git_materialization: Arc<Semaphore>,
    git_segment_ingest: Arc<Semaphore>,
    object_store: Arc<Semaphore>,
    git_materialization_wait: Duration,
    git_command_timeout: Duration,
    git_storage_limits: GitStorageLimits,
}

impl RuntimeBudgets {
    pub(crate) fn from_env() -> anyhow::Result<Self> {
        Ok(Self::from_config(RuntimeBudgetConfig::from_env()?))
    }

    pub(crate) fn from_config(config: RuntimeBudgetConfig) -> Self {
        Self {
            receive_pack: Arc::new(Semaphore::new(config.receive_pack_concurrency)),
            upload_pack: Arc::new(Semaphore::new(config.upload_pack_concurrency)),
            git_materialization: Arc::new(Semaphore::new(config.git_materialization_concurrency)),
            git_segment_ingest: Arc::new(Semaphore::new(config.git_segment_ingest_concurrency)),
            object_store: Arc::new(Semaphore::new(config.object_store_concurrency)),
            git_materialization_wait: config.git_materialization_wait,
            git_command_timeout: config.git_command_timeout,
            git_storage_limits: config.git_storage_limits,
        }
    }

    pub(crate) fn try_receive_pack(&self) -> Result<RuntimePermit, ApiError> {
        self.try_acquire(&self.receive_pack, "Git receive-pack")
    }

    pub(crate) fn try_upload_pack(&self) -> Result<RuntimePermit, ApiError> {
        self.try_acquire(&self.upload_pack, "Git upload-pack")
    }

    pub(crate) fn acquire_git_materialization(&self) -> Result<RuntimePermit, ApiError> {
        let started_at = Instant::now();
        let mut backoff = Duration::from_millis(2);
        loop {
            match self.git_materialization.clone().try_acquire_owned() {
                Ok(permit) => {
                    let waited = started_at.elapsed();
                    if !waited.is_zero() {
                        tracing::debug!(
                            waited_us = waited.as_micros(),
                            "Git materialization capacity acquired"
                        );
                    }
                    return Ok(RuntimePermit { _permit: permit });
                }
                Err(_) if started_at.elapsed() < self.git_materialization_wait => {
                    let remaining = self
                        .git_materialization_wait
                        .saturating_sub(started_at.elapsed());
                    std::thread::sleep(backoff.min(remaining));
                    backoff = (backoff * 2).min(Duration::from_millis(25));
                }
                Err(_) => {
                    tracing::warn!(
                        operation = "Git materialization",
                        waited_us = started_at.elapsed().as_micros(),
                        "runtime capacity permit rejected"
                    );
                    return Err(ApiError::too_many_requests(
                        "Git materialization capacity is exhausted; retry later",
                    ));
                }
            }
        }
    }

    pub(crate) fn try_git_segment_ingest(&self) -> Result<RuntimePermit, ApiError> {
        self.try_acquire(&self.git_segment_ingest, "Git segment ingest")
    }

    pub(crate) fn try_object_store(&self, operation: &str) -> Result<RuntimePermit, ApiError> {
        self.try_acquire(&self.object_store, operation)
    }

    pub(crate) fn git_command_timeout(&self) -> Duration {
        self.git_command_timeout
    }

    pub(crate) fn git_storage_limits(&self) -> GitStorageLimits {
        self.git_storage_limits
    }

    pub(crate) fn default_git_command_timeout() -> Duration {
        static DEFAULT_GIT_COMMAND_TIMEOUT: OnceLock<Duration> = OnceLock::new();
        // Runtime env is boot-time config. Tests that need per-case values should
        // use RuntimeBudgets::from_config instead of mutating env after this cache initializes.
        *DEFAULT_GIT_COMMAND_TIMEOUT.get_or_init(|| {
            RuntimeBudgetConfig::from_env()
                .map(|config| config.git_command_timeout)
                .unwrap_or_else(|_| RuntimeBudgetConfig::default().git_command_timeout)
        })
    }

    fn try_acquire(
        &self,
        semaphore: &Arc<Semaphore>,
        operation: &str,
    ) -> Result<RuntimePermit, ApiError> {
        semaphore
            .clone()
            .try_acquire_owned()
            .map(|permit| RuntimePermit { _permit: permit })
            .map_err(|_| {
                tracing::warn!(operation, "runtime capacity permit rejected");
                ApiError::too_many_requests(format!(
                    "{operation} capacity is exhausted; retry later"
                ))
            })
    }

    fn check_object_size(
        &self,
        operation: &str,
        key: &str,
        bytes: usize,
    ) -> Result<(), ObjectStoreError> {
        ensure_object_size(
            operation,
            key,
            bytes,
            self.git_storage_limits.max_object_bytes(),
        )
    }
}

fn git_materialization_wait_from_env() -> anyhow::Result<Duration> {
    let millis = parse_u64_env(
        GIT_MATERIALIZATION_WAIT_MILLIS_ENV,
        DEFAULT_GIT_MATERIALIZATION_WAIT_MILLIS,
    )?;
    if millis > MAX_GIT_MATERIALIZATION_WAIT_MILLIS {
        anyhow::bail!(
            "{GIT_MATERIALIZATION_WAIT_MILLIS_ENV} must be at most {MAX_GIT_MATERIALIZATION_WAIT_MILLIS}"
        );
    }
    Ok(Duration::from_millis(millis))
}

pub(crate) struct RuntimePermit {
    _permit: OwnedSemaphorePermit,
}

pub(crate) struct BudgetedObjectStore {
    inner: Arc<dyn ObjectStore>,
    budgets: Arc<RuntimeBudgets>,
}

impl BudgetedObjectStore {
    pub(crate) fn new(inner: Arc<dyn ObjectStore>, budgets: Arc<RuntimeBudgets>) -> Self {
        Self { inner, budgets }
    }
}

impl ObjectStore for BudgetedObjectStore {
    fn put(&self, key: &str, bytes: &[u8]) -> Result<(), ObjectStoreError> {
        self.budgets.check_object_size("write", key, bytes.len())?;
        let _permit = self
            .budgets
            .try_object_store("object store write")
            .map_err(|error| {
                ObjectStoreError::capacity_exhausted(error.into_operator_diagnostic())
            })?;
        let started = Instant::now();
        let result = self.inner.put(key, bytes);
        tracing::info!(
            operation = "put",
            bytes = bytes.len(),
            elapsed_us = started.elapsed().as_micros(),
            success = result.is_ok(),
            "object store operation timing"
        );
        result
    }

    fn get(&self, key: &str) -> Result<Vec<u8>, ObjectStoreError> {
        let _permit = self
            .budgets
            .try_object_store("object store read")
            .map_err(|error| {
                ObjectStoreError::capacity_exhausted(error.into_operator_diagnostic())
            })?;
        let started = Instant::now();
        let result = self
            .inner
            .get_bounded(key, self.budgets.git_storage_limits.max_object_bytes());
        match result {
            Ok(bytes) => {
                tracing::info!(
                    operation = "get",
                    bytes = bytes.len(),
                    elapsed_us = started.elapsed().as_micros(),
                    success = true,
                    "object store operation timing"
                );
                Ok(bytes)
            }
            Err(error) => {
                tracing::info!(
                    operation = "get",
                    bytes = 0,
                    elapsed_us = started.elapsed().as_micros(),
                    success = false,
                    "object store operation timing"
                );
                Err(error)
            }
        }
    }

    fn delete(&self, key: &str) -> Result<(), ObjectStoreError> {
        let _permit = self
            .budgets
            .try_object_store("object store delete")
            .map_err(|error| {
                ObjectStoreError::capacity_exhausted(error.into_operator_diagnostic())
            })?;
        let started = Instant::now();
        let result = self.inner.delete(key);
        tracing::info!(
            operation = "delete",
            elapsed_us = started.elapsed().as_micros(),
            success = result.is_ok(),
            "object store operation timing"
        );
        result
    }

    fn readiness_check(&self) -> Result<(), ObjectStoreError> {
        self.inner.readiness_check()
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
