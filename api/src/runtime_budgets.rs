use crate::{
    error::ApiError,
    object_store::{ObjectStore, ensure_object_size},
};
use std::{sync::Arc, time::Duration};
use tokio::sync::{OwnedSemaphorePermit, Semaphore};

const DEFAULT_RECEIVE_PACK_CONCURRENCY: usize = 4;
const DEFAULT_UPLOAD_PACK_CONCURRENCY: usize = 8;
const DEFAULT_PROJECTION_BUILD_CONCURRENCY: usize = 2;
const DEFAULT_OBJECT_STORE_CONCURRENCY: usize = 16;
const DEFAULT_GIT_COMMAND_TIMEOUT_SECS: u64 = 30;
const DEFAULT_OBJECT_STORE_MAX_BYTES: usize = 128 * 1024 * 1024;

const RECEIVE_PACK_CONCURRENCY_ENV: &str = "SCOPE_GIT_RECEIVE_PACK_CONCURRENCY";
const UPLOAD_PACK_CONCURRENCY_ENV: &str = "SCOPE_GIT_UPLOAD_PACK_CONCURRENCY";
const PROJECTION_BUILD_CONCURRENCY_ENV: &str = "SCOPE_GIT_PROJECTION_BUILD_CONCURRENCY";
const OBJECT_STORE_CONCURRENCY_ENV: &str = "SCOPE_OBJECT_STORE_CONCURRENCY";
const GIT_COMMAND_TIMEOUT_SECS_ENV: &str = "SCOPE_GIT_COMMAND_TIMEOUT_SECS";
const OBJECT_STORE_MAX_BYTES_ENV: &str = "SCOPE_OBJECT_STORE_MAX_BYTES";

#[derive(Clone, Debug)]
pub(crate) struct RuntimeBudgetConfig {
    pub(crate) receive_pack_concurrency: usize,
    pub(crate) upload_pack_concurrency: usize,
    pub(crate) projection_build_concurrency: usize,
    pub(crate) object_store_concurrency: usize,
    pub(crate) git_command_timeout: Duration,
    pub(crate) object_store_max_bytes: usize,
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
            projection_build_concurrency: parse_usize_env(
                PROJECTION_BUILD_CONCURRENCY_ENV,
                DEFAULT_PROJECTION_BUILD_CONCURRENCY,
            )?,
            object_store_concurrency: parse_usize_env(
                OBJECT_STORE_CONCURRENCY_ENV,
                DEFAULT_OBJECT_STORE_CONCURRENCY,
            )?,
            git_command_timeout: Duration::from_secs(parse_u64_env(
                GIT_COMMAND_TIMEOUT_SECS_ENV,
                DEFAULT_GIT_COMMAND_TIMEOUT_SECS,
            )?),
            object_store_max_bytes: parse_usize_env(
                OBJECT_STORE_MAX_BYTES_ENV,
                DEFAULT_OBJECT_STORE_MAX_BYTES,
            )?,
        })
    }
}

impl Default for RuntimeBudgetConfig {
    fn default() -> Self {
        Self {
            receive_pack_concurrency: DEFAULT_RECEIVE_PACK_CONCURRENCY,
            upload_pack_concurrency: DEFAULT_UPLOAD_PACK_CONCURRENCY,
            projection_build_concurrency: DEFAULT_PROJECTION_BUILD_CONCURRENCY,
            object_store_concurrency: DEFAULT_OBJECT_STORE_CONCURRENCY,
            git_command_timeout: Duration::from_secs(DEFAULT_GIT_COMMAND_TIMEOUT_SECS),
            object_store_max_bytes: DEFAULT_OBJECT_STORE_MAX_BYTES,
        }
    }
}

pub(crate) struct RuntimeBudgets {
    receive_pack: Arc<Semaphore>,
    upload_pack: Arc<Semaphore>,
    projection_build: Arc<Semaphore>,
    object_store: Arc<Semaphore>,
    object_store_concurrency: usize,
    git_command_timeout: Duration,
    object_store_max_bytes: usize,
}

impl RuntimeBudgets {
    pub(crate) fn from_env() -> anyhow::Result<Self> {
        Ok(Self::from_config(RuntimeBudgetConfig::from_env()?))
    }

    pub(crate) fn from_config(config: RuntimeBudgetConfig) -> Self {
        Self {
            receive_pack: Arc::new(Semaphore::new(config.receive_pack_concurrency)),
            upload_pack: Arc::new(Semaphore::new(config.upload_pack_concurrency)),
            projection_build: Arc::new(Semaphore::new(config.projection_build_concurrency)),
            object_store: Arc::new(Semaphore::new(config.object_store_concurrency)),
            object_store_concurrency: config.object_store_concurrency,
            git_command_timeout: config.git_command_timeout,
            object_store_max_bytes: config.object_store_max_bytes,
        }
    }

    pub(crate) fn try_receive_pack(&self) -> Result<RuntimePermit, ApiError> {
        self.try_acquire(&self.receive_pack, "Git receive-pack")
    }

    pub(crate) fn try_upload_pack(&self) -> Result<RuntimePermit, ApiError> {
        self.try_acquire(&self.upload_pack, "Git upload-pack")
    }

    pub(crate) fn try_projection_build(&self) -> Result<RuntimePermit, ApiError> {
        self.try_acquire(&self.projection_build, "Git projection build")
    }

    pub(crate) fn try_object_store(&self, operation: &str) -> Result<RuntimePermit, ApiError> {
        self.try_acquire(&self.object_store, operation)
    }

    pub(crate) fn git_command_timeout(&self) -> Duration {
        self.git_command_timeout
    }

    pub(crate) fn object_store_concurrency(&self) -> usize {
        self.object_store_concurrency
    }

    pub(crate) fn default_git_command_timeout() -> Duration {
        RuntimeBudgetConfig::from_env()
            .map(|config| config.git_command_timeout)
            .unwrap_or_else(|_| RuntimeBudgetConfig::default().git_command_timeout)
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
                ApiError::too_many_requests(format!(
                    "{operation} capacity is exhausted; retry later"
                ))
            })
    }

    fn check_object_size(&self, operation: &str, key: &str, bytes: usize) -> Result<(), ApiError> {
        ensure_object_size(operation, key, bytes, self.object_store_max_bytes)
    }
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
    fn put(&self, key: &str, bytes: &[u8]) -> Result<(), ApiError> {
        self.budgets.check_object_size("write", key, bytes.len())?;
        let _permit = self.budgets.try_object_store("object store write")?;
        self.inner.put(key, bytes)
    }

    fn get(&self, key: &str) -> Result<Vec<u8>, ApiError> {
        let _permit = self.budgets.try_object_store("object store read")?;
        let bytes = self
            .inner
            .get_bounded(key, self.budgets.object_store_max_bytes)?;
        self.budgets.check_object_size("read", key, bytes.len())?;
        Ok(bytes)
    }

    fn delete(&self, key: &str) -> Result<(), ApiError> {
        let _permit = self.budgets.try_object_store("object store delete")?;
        self.inner.delete(key)
    }

    fn readiness_check(&self) -> Result<(), ApiError> {
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
