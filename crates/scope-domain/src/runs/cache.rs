use super::{
    run::PinnedContainerImage,
    workflow::{WorkflowJobId, WorkflowPath},
};
use crate::error::DomainError;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::path::{Component, Path};
use thiserror::Error;

pub const MAX_WORKFLOW_CACHE_NAME_BYTES: usize = 64;
pub const MAX_WORKFLOW_CACHE_PATH_BYTES: usize = 1024;
pub const CACHE_IDENTITY_FORMAT: &str = "scope-cache-v3";

const RESERVED_CACHE_NAME_PREFIX: &str = "scope-";
const RESERVED_CACHE_PATHS: &[&str] = &["/scope-steps", "/scope-step.log", "/scope-active-step"];

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
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
pub struct WorkflowCache {
    name: String,
    path: String,
}

impl WorkflowCache {
    pub fn new(name: impl Into<String>, path: impl Into<String>) -> Result<Self, CacheError> {
        let name = name.into();
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
        Ok(Self { name, path })
    }

    pub fn as_str(&self) -> &str {
        &self.name
    }

    pub fn mount_path(&self) -> &str {
        &self.path
    }
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
    RunnerProtocolCanary,
}

impl CacheNamespace {
    pub fn workflow(workflow_path: &WorkflowPath, job_key: &WorkflowJobId) -> Self {
        Self::Workflow {
            workflow_path: workflow_path.as_str().to_string(),
            job_key: job_key.as_str().to_string(),
        }
    }

    fn validate(&self) -> Result<(), DomainError> {
        if let Self::Workflow {
            workflow_path,
            job_key,
        } = self
        {
            WorkflowPath::parse(workflow_path.clone()).map_err(DomainError::invalid_input)?;
            WorkflowJobId::parse(job_key.clone()).map_err(DomainError::invalid_input)?;
        }
        Ok(())
    }

    pub fn kind(&self) -> &'static str {
        match self {
            Self::Workflow { .. } => "workflow",
            Self::RunnerProtocolCanary => "runner-protocol-canary",
        }
    }

    fn digest_components(&self) -> Vec<&str> {
        match self {
            Self::Workflow {
                workflow_path,
                job_key,
            } => vec!["workflow", workflow_path, job_key],
            Self::RunnerProtocolCanary => vec!["runner-protocol-canary"],
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
    image_digest: String,
    platform: CachePlatform,
}

impl CacheIdentity {
    pub fn new(
        repository_id: impl Into<String>,
        namespace: CacheNamespace,
        cache: WorkflowCache,
        image: &PinnedContainerImage,
        platform: CachePlatform,
    ) -> Result<Self, DomainError> {
        let repository_id = repository_id.into();
        if repository_id.trim().is_empty() {
            return Err(DomainError::invalid_input(
                "cache identity repository id is required",
            ));
        }
        namespace.validate()?;
        Ok(Self {
            repository_id,
            namespace,
            cache,
            image_digest: image.digest().to_string(),
            platform,
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

    pub fn image_digest(&self) -> &str {
        &self.image_digest
    }

    pub fn platform(&self) -> CachePlatform {
        self.platform
    }

    /// Stable, storage-agnostic key for translating this semantic identity.
    pub fn digest(&self) -> String {
        let mut digest = Sha256::new();
        let components = [CACHE_IDENTITY_FORMAT, self.repository_id.as_str()]
            .into_iter()
            .chain(self.namespace.digest_components())
            .chain([
                self.cache.as_str(),
                self.cache.mount_path(),
                self.image_digest.as_str(),
                self.platform.as_str(),
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

    fn image(digest: char) -> PinnedContainerImage {
        PinnedContainerImage::parse(format!(
            "registry.example/scope@sha256:{}",
            digest.to_string().repeat(64)
        ))
        .unwrap()
    }

    fn workflow_namespace(path: &str, job: &str) -> CacheNamespace {
        CacheNamespace::workflow(
            &WorkflowPath::parse(path).unwrap(),
            &WorkflowJobId::parse(job).unwrap(),
        )
    }

    fn cache(name: &str) -> WorkflowCache {
        WorkflowCache::new(name, format!("/scope/cache/{name}")).unwrap()
    }

    #[test]
    fn workflow_cache_names_and_mount_paths_are_validated() {
        let cache = WorkflowCache::new("cargo-target", "/workspace/target").unwrap();
        assert_eq!(cache.as_str(), "cargo-target");
        assert_eq!(cache.mount_path(), "/workspace/target");

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
                WorkflowCache::new(invalid, "/scope/cache/valid").is_err(),
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
        ] {
            assert!(WorkflowCache::new("cargo", invalid).is_err(), "{invalid}");
        }
    }

    #[test]
    fn identity_is_partitioned_by_every_semantic_component() {
        let workflow_cache = cache("cargo");
        let base = CacheIdentity::new(
            "repo-1",
            workflow_namespace("/.scope/runs/test.yml", "checks"),
            workflow_cache.clone(),
            &image('a'),
            CachePlatform::LinuxAmd64,
        )
        .unwrap();
        let other_repo = CacheIdentity::new(
            "repo-2",
            workflow_namespace("/.scope/runs/test.yml", "checks"),
            workflow_cache.clone(),
            &image('a'),
            CachePlatform::LinuxAmd64,
        )
        .unwrap();
        let other_cache = CacheIdentity::new(
            "repo-1",
            workflow_namespace("/.scope/runs/test.yml", "checks"),
            cache("cargo-target"),
            &image('a'),
            CachePlatform::LinuxAmd64,
        )
        .unwrap();
        let other_image = CacheIdentity::new(
            "repo-1",
            workflow_namespace("/.scope/runs/test.yml", "checks"),
            workflow_cache.clone(),
            &image('b'),
            CachePlatform::LinuxAmd64,
        )
        .unwrap();
        let other_workflow = CacheIdentity::new(
            "repo-1",
            workflow_namespace("/.scope/runs/release.yml", "checks"),
            cache("cargo"),
            &image('a'),
            CachePlatform::LinuxAmd64,
        )
        .unwrap();
        let other_job = CacheIdentity::new(
            "repo-1",
            workflow_namespace("/.scope/runs/test.yml", "release"),
            cache("cargo"),
            &image('a'),
            CachePlatform::LinuxAmd64,
        )
        .unwrap();
        let canary = CacheIdentity::new(
            "repo-1",
            CacheNamespace::RunnerProtocolCanary,
            cache("cargo"),
            &image('a'),
            CachePlatform::LinuxAmd64,
        )
        .unwrap();
        let other_path = CacheIdentity::new(
            "repo-1",
            workflow_namespace("/.scope/runs/test.yml", "checks"),
            WorkflowCache::new("cargo", "/different/cache").unwrap(),
            &image('a'),
            CachePlatform::LinuxAmd64,
        )
        .unwrap();

        assert_eq!(base.digest(), base.digest());
        assert_eq!(base.digest().len(), 64);
        assert_ne!(base.digest(), other_repo.digest());
        assert_ne!(base.digest(), other_cache.digest());
        assert_ne!(base.digest(), other_image.digest());
        assert_ne!(base.digest(), other_workflow.digest());
        assert_ne!(base.digest(), other_job.digest());
        assert_ne!(base.digest(), canary.digest());
        assert_ne!(base.digest(), other_path.digest());
        assert!(
            CacheIdentity::new(
                " ",
                workflow_namespace("/.scope/runs/test.yml", "checks"),
                cache("cargo"),
                &image('a'),
                CachePlatform::LinuxAmd64,
            )
            .is_err()
        );
    }
}
