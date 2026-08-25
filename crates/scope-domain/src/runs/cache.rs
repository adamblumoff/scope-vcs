use super::workflow::{WorkflowJobId, WorkflowPath};
use crate::error::DomainError;
use scope_cache_domain::MAX_CACHE_OBJECT_BYTES;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::path::{Component, Path};
use thiserror::Error;

pub const MAX_WORKFLOW_CACHE_NAME_BYTES: usize = 64;
pub const MAX_WORKFLOW_CACHE_PATH_BYTES: usize = 1024;
pub const MAX_WORKFLOW_CACHE_FORMAT_BYTES: usize = 64;
pub const MAX_WORKFLOW_CACHE_KEY_INPUTS: usize = 128;
pub const MAX_WORKFLOW_CACHE_INPUT_PATH_BYTES: usize = 1024;
pub const CACHE_IDENTITY_FORMAT: &str = "scope-cache-v4";
pub const MAX_CACHE_OBSERVATION_DURATION_MS: u64 = 24 * 60 * 60 * 1_000;

const RESERVED_CACHE_NAME_PREFIX: &str = "scope-";
const RESERVED_CACHE_PATHS: &[&str] = &[
    "/scope-steps",
    "/scope-step.log",
    "/scope-active-step",
    "/workspace/target",
];

#[derive(Clone, Debug, Error, PartialEq, Eq)]
pub enum CacheError {
    #[error(
        "workflow cache name must contain between 1 and {MAX_WORKFLOW_CACHE_NAME_BYTES} bytes of lowercase letters, numbers, or single hyphens"
    )]
    InvalidName,
    #[error("workflow cache names beginning with {RESERVED_CACHE_NAME_PREFIX:?} are reserved")]
    ReservedName,
    #[error(
        "workflow cache path must be a normalized Docker-mount-safe absolute path between 1 and {MAX_WORKFLOW_CACHE_PATH_BYTES} bytes"
    )]
    InvalidPath,
    #[error(
        "workflow cache format must contain between 1 and {MAX_WORKFLOW_CACHE_FORMAT_BYTES} bytes of lowercase letters, numbers, or single hyphens"
    )]
    InvalidFormat,
    #[error("workflow cache key cannot contain more than {MAX_WORKFLOW_CACHE_KEY_INPUTS} inputs")]
    TooManyKeyInputs,
    #[error(
        "workflow cache input path must be a normalized repository-relative path between 1 and {MAX_WORKFLOW_CACHE_INPUT_PATH_BYTES} bytes"
    )]
    InvalidInputPath,
    #[error("workflow cache environment input must be a valid shell variable name")]
    InvalidEnvironmentInput,
    #[error("workflow cache key input {0:?} is duplicated")]
    DuplicateKeyInput(String),
}

#[derive(Clone, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
pub struct CacheKeyInputs {
    files: Vec<String>,
    environment: Vec<String>,
    source: bool,
}

impl CacheKeyInputs {
    pub fn new(
        mut files: Vec<String>,
        mut environment: Vec<String>,
        source: bool,
    ) -> Result<Self, CacheError> {
        if files
            .len()
            .saturating_add(environment.len())
            .saturating_add(usize::from(source))
            > MAX_WORKFLOW_CACHE_KEY_INPUTS
        {
            return Err(CacheError::TooManyKeyInputs);
        }
        for path in &files {
            let parsed = Path::new(path);
            if path.is_empty()
                || path.len() > MAX_WORKFLOW_CACHE_INPUT_PATH_BYTES
                || parsed.is_absolute()
                || path
                    .bytes()
                    .any(|byte| matches!(byte, b'\0' | b'\r' | b'\n'))
                || path
                    .split('/')
                    .any(|segment| segment.is_empty() || matches!(segment, "." | ".."))
                || parsed
                    .components()
                    .any(|component| !matches!(component, Component::Normal(_)))
            {
                return Err(CacheError::InvalidInputPath);
            }
        }
        for name in &environment {
            let mut bytes = name.bytes();
            if name.is_empty()
                || !bytes
                    .next()
                    .is_some_and(|byte| byte.is_ascii_alphabetic() || byte == b'_')
                || !bytes.all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
            {
                return Err(CacheError::InvalidEnvironmentInput);
            }
        }
        files.sort();
        environment.sort();
        if let Some(duplicate) = files.windows(2).find(|pair| pair[0] == pair[1]) {
            return Err(CacheError::DuplicateKeyInput(duplicate[0].clone()));
        }
        if let Some(duplicate) = environment.windows(2).find(|pair| pair[0] == pair[1]) {
            return Err(CacheError::DuplicateKeyInput(duplicate[0].clone()));
        }
        Ok(Self {
            files,
            environment,
            source,
        })
    }

    pub fn files(&self) -> &[String] {
        &self.files
    }

    pub fn environment(&self) -> &[String] {
        &self.environment
    }

    pub fn includes_source(&self) -> bool {
        self.source
    }
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
pub struct WorkflowCache {
    name: String,
    path: String,
    format: String,
    compatibility: CacheKeyInputs,
    exact: CacheKeyInputs,
}

impl WorkflowCache {
    pub fn new(
        name: impl Into<String>,
        path: impl Into<String>,
        format: impl Into<String>,
        compatibility: CacheKeyInputs,
        exact: CacheKeyInputs,
    ) -> Result<Self, CacheError> {
        let name = name.into();
        validate_cache_name(&name)?;
        let path = path.into();
        let parsed = Path::new(&path);
        if path.is_empty()
            || path.len() > MAX_WORKFLOW_CACHE_PATH_BYTES
            || path == "/"
            || !parsed.is_absolute()
            || path
                .bytes()
                .any(|byte| matches!(byte, b'\0' | b',' | b'"' | b'\r' | b'\n'))
            || path
                .split('/')
                .skip(1)
                .any(|segment| segment.is_empty() || matches!(segment, "." | ".."))
            || parsed
                .components()
                .any(|component| !matches!(component, Component::RootDir | Component::Normal(_)))
            || parsed == Path::new("/workspace")
            || RESERVED_CACHE_PATHS.iter().any(|reserved| {
                let reserved = Path::new(reserved);
                parsed.starts_with(reserved) || reserved.starts_with(parsed)
            })
        {
            return Err(CacheError::InvalidPath);
        }
        let format = format.into();
        if format.is_empty()
            || format.len() > MAX_WORKFLOW_CACHE_FORMAT_BYTES
            || format.starts_with('-')
            || format.ends_with('-')
            || format.contains("--")
            || !format
                .bytes()
                .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
        {
            return Err(CacheError::InvalidFormat);
        }
        Ok(Self {
            name,
            path,
            format,
            compatibility,
            exact,
        })
    }

    pub fn as_str(&self) -> &str {
        &self.name
    }

    pub fn mount_path(&self) -> &str {
        &self.path
    }

    pub fn format(&self) -> &str {
        &self.format
    }

    pub fn compatibility_inputs(&self) -> &CacheKeyInputs {
        &self.compatibility
    }

    pub fn exact_inputs(&self) -> &CacheKeyInputs {
        &self.exact
    }
}

fn validate_cache_name(name: &str) -> Result<(), CacheError> {
    if name.is_empty()
        || name.len() > MAX_WORKFLOW_CACHE_NAME_BYTES
        || name.starts_with('-')
        || name.ends_with('-')
        || name.contains("--")
        || !name
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
    {
        return Err(CacheError::InvalidName);
    }
    if name.starts_with(RESERVED_CACHE_NAME_PREFIX) {
        return Err(CacheError::ReservedName);
    }
    Ok(())
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum CacheColdReason {
    MetadataMissing,
    MetadataInvalid,
    MetadataNotReady,
    VolumeMissing,
    VolumeInvalid,
    BackingDirectoryMissing,
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum CachePreparation {
    Exact,
    Compatible,
    Cold { reason: CacheColdReason },
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum CacheFinalState {
    Pending,
    Ready,
    Evicted,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AttemptCacheSetupObservation {
    pub attempt_id: String,
    pub authorization_ms: u64,
    pub wall_ms: u64,
}

impl AttemptCacheSetupObservation {
    pub fn new(
        attempt_id: impl Into<String>,
        authorization_ms: u64,
        wall_ms: u64,
    ) -> Result<Self, DomainError> {
        let attempt_id = attempt_id.into();
        if attempt_id.trim().is_empty() {
            return Err(DomainError::invalid_input(
                "cache setup observation attempt id is required",
            ));
        }
        validate_observation_duration(authorization_ms)?;
        validate_observation_duration(wall_ms)?;
        if authorization_ms > wall_ms {
            return Err(DomainError::invalid_input(
                "cache authorization duration cannot exceed cache setup wall duration",
            ));
        }
        Ok(Self {
            attempt_id,
            authorization_ms,
            wall_ms,
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AttemptCachePreparationTiming {
    pub key_ms: u64,
    pub metadata_ms: u64,
    pub size_bytes: u64,
    pub download_verify_ms: u64,
    pub sync_ms: u64,
    pub extraction_ms: u64,
    pub prepare_ms: u64,
}

impl AttemptCachePreparationTiming {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        key_ms: u64,
        metadata_ms: u64,
        size_bytes: u64,
        download_verify_ms: u64,
        sync_ms: u64,
        extraction_ms: u64,
        prepare_ms: u64,
    ) -> Result<Self, DomainError> {
        for duration in [
            key_ms,
            metadata_ms,
            download_verify_ms,
            sync_ms,
            extraction_ms,
            prepare_ms,
        ] {
            validate_observation_duration(duration)?;
        }
        if size_bytes > MAX_CACHE_OBJECT_BYTES {
            return Err(DomainError::invalid_input(
                "cache observation size exceeds the maximum cache object size",
            ));
        }
        let derived_prepare_ms = key_ms
            .checked_add(metadata_ms)
            .and_then(|total| total.checked_add(download_verify_ms))
            .and_then(|total| total.checked_add(sync_ms))
            .and_then(|total| total.checked_add(extraction_ms))
            .ok_or_else(|| DomainError::invalid_input("cache preparation duration overflow"))?;
        if prepare_ms != derived_prepare_ms {
            return Err(DomainError::invalid_input(
                "cache preparation duration must equal the sum of its measured phases",
            ));
        }
        Ok(Self {
            key_ms,
            metadata_ms,
            size_bytes,
            download_verify_ms,
            sync_ms,
            extraction_ms,
            prepare_ms,
        })
    }
}

/// Durable facts observed by a runner for one cache during one attempt.
///
/// The workflow namespace is supplied by the claimed attempt, not by the runner
/// report, so a report cannot move a cache observation across jobs.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AttemptCacheObservation {
    pub attempt_id: String,
    pub workflow_path: WorkflowPath,
    pub job_key: WorkflowJobId,
    pub cache_name: String,
    pub identity_digest: String,
    pub preparation: CachePreparation,
    pub timing: AttemptCachePreparationTiming,
    pub final_state: CacheFinalState,
    pub finalize_ms: Option<u64>,
}

impl AttemptCacheObservation {
    pub fn prepared(
        attempt_id: impl Into<String>,
        workflow_path: WorkflowPath,
        job_key: WorkflowJobId,
        cache_name: impl Into<String>,
        identity_digest: impl Into<String>,
        preparation: CachePreparation,
        timing: AttemptCachePreparationTiming,
    ) -> Result<Self, DomainError> {
        let attempt_id = attempt_id.into();
        if attempt_id.trim().is_empty() {
            return Err(DomainError::invalid_input(
                "cache observation attempt id is required",
            ));
        }
        let cache_name = cache_name.into();
        validate_cache_name(&cache_name).map_err(DomainError::invalid_input)?;
        let identity_digest = identity_digest.into();
        if identity_digest.len() != 64
            || !identity_digest
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        {
            return Err(DomainError::invalid_input(
                "cache observation identity digest must be 64 lowercase hexadecimal characters",
            ));
        }
        AttemptCachePreparationTiming::new(
            timing.key_ms,
            timing.metadata_ms,
            timing.size_bytes,
            timing.download_verify_ms,
            timing.sync_ms,
            timing.extraction_ms,
            timing.prepare_ms,
        )?;
        Ok(Self {
            attempt_id,
            workflow_path,
            job_key,
            cache_name,
            identity_digest,
            preparation,
            timing,
            final_state: CacheFinalState::Pending,
            finalize_ms: None,
        })
    }

    #[allow(clippy::too_many_arguments)]
    pub fn restore(
        attempt_id: impl Into<String>,
        workflow_path: WorkflowPath,
        job_key: WorkflowJobId,
        cache_name: impl Into<String>,
        identity_digest: impl Into<String>,
        preparation: CachePreparation,
        timing: AttemptCachePreparationTiming,
        final_state: CacheFinalState,
        finalize_ms: Option<u64>,
    ) -> Result<Self, DomainError> {
        let mut observation = Self::prepared(
            attempt_id,
            workflow_path,
            job_key,
            cache_name,
            identity_digest,
            preparation,
            timing,
        )?;
        match (final_state, finalize_ms) {
            (CacheFinalState::Pending, None) => {}
            (CacheFinalState::Ready | CacheFinalState::Evicted, Some(duration)) => {
                observation.finalize(final_state, duration)?;
            }
            _ => {
                return Err(DomainError::invalid_input(
                    "cache final state and duration are inconsistent",
                ));
            }
        }
        Ok(observation)
    }

    /// Exact retries are idempotent; a different terminal report is a conflict.
    pub fn finalize(
        &mut self,
        state: CacheFinalState,
        finalize_ms: u64,
    ) -> Result<bool, DomainError> {
        if state == CacheFinalState::Pending {
            return Err(DomainError::invalid_input(
                "cache finalization must be ready or evicted",
            ));
        }
        validate_observation_duration(finalize_ms)?;
        match (self.final_state, self.finalize_ms) {
            (CacheFinalState::Pending, None) => {
                self.final_state = state;
                self.finalize_ms = Some(finalize_ms);
                Ok(true)
            }
            (existing_state, Some(existing_ms))
                if existing_state == state && existing_ms == finalize_ms =>
            {
                Ok(false)
            }
            _ => Err(DomainError::conflict(
                "cache observation already finalized with different facts",
            )),
        }
    }

    pub fn has_same_preparation(&self, other: &Self) -> bool {
        self.attempt_id == other.attempt_id
            && self.workflow_path == other.workflow_path
            && self.job_key == other.job_key
            && self.cache_name == other.cache_name
            && self.identity_digest == other.identity_digest
            && self.preparation == other.preparation
            && self.timing == other.timing
    }
}

fn validate_observation_duration(duration_ms: u64) -> Result<(), DomainError> {
    if duration_ms > MAX_CACHE_OBSERVATION_DURATION_MS {
        return Err(DomainError::invalid_input(
            "cache observation duration exceeds the maximum job duration",
        ));
    }
    Ok(())
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum CachePlatform {
    LinuxAmd64,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Hash, Serialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum CacheNamespace {
    Workflow {
        workflow_path: String,
        job_key: String,
    },
}

impl CacheNamespace {
    pub fn workflow(workflow_path: &WorkflowPath, job_key: &WorkflowJobId) -> Self {
        Self::Workflow {
            workflow_path: workflow_path.as_str().to_string(),
            job_key: job_key.as_str().to_string(),
        }
    }

    fn validate(&self) -> Result<(), DomainError> {
        let Self::Workflow {
            workflow_path,
            job_key,
        } = self;
        WorkflowPath::parse(workflow_path.clone()).map_err(DomainError::invalid_input)?;
        WorkflowJobId::parse(job_key.clone()).map_err(DomainError::invalid_input)?;
        Ok(())
    }

    pub fn kind(&self) -> &'static str {
        match self {
            Self::Workflow { .. } => "workflow",
        }
    }

    fn digest_components(&self) -> Vec<&str> {
        match self {
            Self::Workflow {
                workflow_path,
                job_key,
            } => vec!["workflow", workflow_path, job_key],
        }
    }
}

impl CachePlatform {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::LinuxAmd64 => "linux/amd64",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct CacheIdentity {
    repository_id: String,
    namespace: CacheNamespace,
    cache: WorkflowCache,
    platform: CachePlatform,
    compatibility_inputs_digest: String,
    exact_inputs_digest: String,
}

impl CacheIdentity {
    pub fn new(
        repository_id: impl Into<String>,
        namespace: CacheNamespace,
        cache: WorkflowCache,
        platform: CachePlatform,
        compatibility_inputs_digest: impl Into<String>,
        exact_inputs_digest: impl Into<String>,
    ) -> Result<Self, DomainError> {
        let repository_id = repository_id.into();
        if repository_id.trim().is_empty() {
            return Err(DomainError::invalid_input(
                "cache identity repository id is required",
            ));
        }
        namespace.validate()?;
        let compatibility_inputs_digest = compatibility_inputs_digest.into();
        let exact_inputs_digest = exact_inputs_digest.into();
        for digest in [&compatibility_inputs_digest, &exact_inputs_digest] {
            if digest.len() != 64
                || !digest
                    .bytes()
                    .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
            {
                return Err(DomainError::invalid_input(
                    "cache input digest must be 64 lowercase hexadecimal characters",
                ));
            }
        }
        Ok(Self {
            repository_id,
            namespace,
            cache,
            platform,
            compatibility_inputs_digest,
            exact_inputs_digest,
        })
    }

    pub fn repository_id(&self) -> &str {
        &self.repository_id
    }

    pub fn cache(&self) -> &WorkflowCache {
        &self.cache
    }

    pub fn namespace(&self) -> &CacheNamespace {
        &self.namespace
    }

    pub fn platform(&self) -> CachePlatform {
        self.platform
    }

    /// Stable, storage-agnostic key for translating this semantic identity.
    pub fn compatibility_group_digest(&self) -> String {
        self.digest_with_inputs("compatibility", &self.compatibility_inputs_digest)
    }

    pub fn exact_digest(&self) -> String {
        let group = self.compatibility_group_digest();
        self.digest_with_inputs("exact", &format!("{group}:{}", self.exact_inputs_digest))
    }

    fn digest_with_inputs(&self, kind: &str, inputs: &str) -> String {
        let mut digest = Sha256::new();
        let components = [CACHE_IDENTITY_FORMAT, kind, self.repository_id.as_str()]
            .into_iter()
            .chain(self.namespace.digest_components())
            .chain([
                self.cache.as_str(),
                self.cache.mount_path(),
                self.cache.format(),
                self.platform.as_str(),
                inputs,
            ]);
        for component in components {
            digest.update(
                u64::try_from(component.len())
                    .expect("cache identity components fit in u64")
                    .to_be_bytes(),
            );
            digest.update(component.as_bytes());
        }
        hex::encode(digest.finalize())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn workflow_namespace(path: &str, job: &str) -> CacheNamespace {
        CacheNamespace::workflow(
            &WorkflowPath::parse(path).unwrap(),
            &WorkflowJobId::parse(job).unwrap(),
        )
    }

    fn cache(name: &str) -> WorkflowCache {
        WorkflowCache::new(
            name,
            format!("/scope/cache/{name}"),
            "v1",
            CacheKeyInputs::default(),
            CacheKeyInputs::default(),
        )
        .unwrap()
    }

    fn identity(
        repository: &str,
        path: &str,
        job: &str,
        cache: WorkflowCache,
        group: char,
        exact: char,
    ) -> CacheIdentity {
        CacheIdentity::new(
            repository,
            workflow_namespace(path, job),
            cache,
            CachePlatform::LinuxAmd64,
            group.to_string().repeat(64),
            exact.to_string().repeat(64),
        )
        .unwrap()
    }

    #[test]
    fn workflow_cache_names_and_mount_paths_are_validated() {
        let cache = cache("cargo");
        assert_eq!(cache.as_str(), "cargo");
        assert_eq!(cache.mount_path(), "/scope/cache/cargo");

        for invalid in [
            "",
            "Cargo",
            "cargo_target",
            "-cargo",
            "cargo-",
            "cargo--target",
            "scope-internal",
        ] {
            assert!(
                WorkflowCache::new(
                    invalid,
                    "/scope/cache/valid",
                    "v1",
                    Default::default(),
                    Default::default()
                )
                .is_err(),
                "{invalid}"
            );
        }
        for invalid in [
            "",
            "relative",
            "/",
            "/cache/../escape",
            "/cache/./same",
            "/cache,readonly",
            "/cache/\"quoted\"",
            "/cache/nul\0byte",
            "/cache/new\nline",
            "/cache/carriage\rreturn",
            "/workspace/target",
            "/workspace/target/debug",
        ] {
            assert!(
                WorkflowCache::new(
                    "cargo",
                    invalid,
                    "v1",
                    Default::default(),
                    Default::default()
                )
                .is_err(),
                "{invalid}"
            );
        }
    }

    #[test]
    fn identity_is_partitioned_by_every_semantic_component() {
        let workflow_cache = cache("cargo");
        let base = identity(
            "repo-1",
            "/.scope/runs/test.yml",
            "checks",
            workflow_cache.clone(),
            'a',
            'b',
        );
        let other_repo = identity(
            "repo-2",
            "/.scope/runs/test.yml",
            "checks",
            workflow_cache.clone(),
            'a',
            'b',
        );
        let other_cache = identity(
            "repo-1",
            "/.scope/runs/test.yml",
            "checks",
            cache("cargo-target"),
            'a',
            'b',
        );
        let other_workflow = identity(
            "repo-1",
            "/.scope/runs/release.yml",
            "checks",
            cache("cargo"),
            'a',
            'b',
        );
        let other_job = identity(
            "repo-1",
            "/.scope/runs/test.yml",
            "release",
            cache("cargo"),
            'a',
            'b',
        );
        let other_group = identity(
            "repo-1",
            "/.scope/runs/test.yml",
            "checks",
            workflow_cache.clone(),
            'c',
            'b',
        );
        let other_exact = identity(
            "repo-1",
            "/.scope/runs/test.yml",
            "checks",
            workflow_cache,
            'a',
            'c',
        );

        assert_eq!(base.exact_digest(), base.exact_digest());
        assert_eq!(base.exact_digest().len(), 64);
        assert_ne!(base.exact_digest(), other_repo.exact_digest());
        assert_ne!(base.exact_digest(), other_cache.exact_digest());
        assert_ne!(base.exact_digest(), other_workflow.exact_digest());
        assert_ne!(base.exact_digest(), other_job.exact_digest());
        assert_ne!(base.exact_digest(), other_group.exact_digest());
        assert_ne!(base.exact_digest(), other_exact.exact_digest());
        assert_eq!(
            base.compatibility_group_digest(),
            other_exact.compatibility_group_digest()
        );
        assert!(
            CacheIdentity::new(
                " ",
                workflow_namespace("/.scope/runs/test.yml", "checks"),
                cache("cargo"),
                CachePlatform::LinuxAmd64,
                "a".repeat(64),
                "b".repeat(64),
            )
            .is_err()
        );
    }

    #[test]
    fn attempt_cache_observation_accepts_exact_retries_and_rejects_conflicts() {
        let cold_timing = AttemptCachePreparationTiming::new(7, 10, 0, 0, 0, 0, 17).unwrap();
        let mut observation = AttemptCacheObservation::prepared(
            "attempt-1",
            WorkflowPath::parse("/.scope/runs/test.yml").unwrap(),
            WorkflowJobId::parse("checks").unwrap(),
            "cargo",
            "a".repeat(64),
            CachePreparation::Cold {
                reason: CacheColdReason::MetadataMissing,
            },
            cold_timing,
        )
        .unwrap();

        assert!(observation.finalize(CacheFinalState::Ready, 9).unwrap());
        assert!(!observation.finalize(CacheFinalState::Ready, 9).unwrap());
        assert!(observation.finalize(CacheFinalState::Evicted, 9).is_err());
        assert!(
            AttemptCacheObservation::prepared(
                "attempt-1",
                WorkflowPath::parse("/.scope/runs/test.yml").unwrap(),
                WorkflowJobId::parse("checks").unwrap(),
                "cargo",
                "A".repeat(64),
                CachePreparation::Exact,
                AttemptCachePreparationTiming::new(1, 0, 1, 0, 0, 0, 1).unwrap(),
            )
            .is_err()
        );
    }

    #[test]
    fn cache_timing_requires_truthful_phase_totals_and_setup_wall_time() {
        assert!(AttemptCachePreparationTiming::new(1, 2, 3, 4, 5, 6, 17).is_err());
        assert!(
            AttemptCachePreparationTiming::new(1, 2, MAX_CACHE_OBJECT_BYTES + 1, 0, 0, 0, 3,)
                .is_err()
        );
        assert!(AttemptCacheSetupObservation::new("attempt-1", 5, 4).is_err());
        assert_eq!(
            AttemptCacheSetupObservation::new("attempt-1", 4, 5).unwrap(),
            AttemptCacheSetupObservation {
                attempt_id: "attempt-1".to_string(),
                authorization_ms: 4,
                wall_ms: 5,
            }
        );
    }
}
