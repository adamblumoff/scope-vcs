use super::run::PinnedContainerImage;
use crate::error::DomainError;
use serde::Serialize;
use sha2::{Digest, Sha256};
use thiserror::Error;

pub const MAX_WORKFLOW_CACHE_NAME_BYTES: usize = 64;
pub const CACHE_IDENTITY_FORMAT: &str = "scope-cache-v1";

const CACHE_MOUNT_ROOT: &str = "/scope/cache";
const RESERVED_CACHE_NAME_PREFIX: &str = "scope-";

#[derive(Clone, Debug, Error, PartialEq, Eq)]
pub enum CacheError {
    #[error(
        "workflow cache name must contain between 1 and {MAX_WORKFLOW_CACHE_NAME_BYTES} bytes of lowercase letters, numbers, or single hyphens"
    )]
    InvalidName,
    #[error("workflow cache names beginning with {RESERVED_CACHE_NAME_PREFIX:?} are reserved")]
    ReservedName,
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(transparent)]
pub struct WorkflowCache(String);

impl WorkflowCache {
    pub fn parse(name: impl Into<String>) -> Result<Self, CacheError> {
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
        Ok(Self(name))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn mount_path(&self) -> String {
        format!("{CACHE_MOUNT_ROOT}/{}", self.0)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum CachePlatform {
    LinuxAmd64,
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
    cache: WorkflowCache,
    image_digest: String,
    platform: CachePlatform,
}

impl CacheIdentity {
    pub fn new(
        repository_id: impl Into<String>,
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
        Ok(Self {
            repository_id,
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

    pub fn image_digest(&self) -> &str {
        &self.image_digest
    }

    pub fn platform(&self) -> CachePlatform {
        self.platform
    }

    /// Stable, storage-agnostic key for translating this semantic identity.
    pub fn digest(&self) -> String {
        let mut digest = Sha256::new();
        for component in [
            CACHE_IDENTITY_FORMAT,
            self.repository_id.as_str(),
            self.cache.as_str(),
            self.image_digest.as_str(),
            self.platform.as_str(),
        ] {
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

    #[test]
    fn workflow_cache_names_have_one_canonical_mount() {
        let cache = WorkflowCache::parse("cargo-target").unwrap();
        assert_eq!(cache.as_str(), "cargo-target");
        assert_eq!(cache.mount_path(), "/scope/cache/cargo-target");

        for invalid in [
            "",
            "Cargo",
            "cargo_target",
            "-cargo",
            "cargo-",
            "cargo--target",
            "scope-internal",
        ] {
            assert!(WorkflowCache::parse(invalid).is_err(), "{invalid}");
        }
    }

    #[test]
    fn identity_is_partitioned_by_every_semantic_component() {
        let cache = WorkflowCache::parse("cargo").unwrap();
        let base = CacheIdentity::new(
            "repo-1",
            cache.clone(),
            &image('a'),
            CachePlatform::LinuxAmd64,
        )
        .unwrap();
        let other_repo = CacheIdentity::new(
            "repo-2",
            cache.clone(),
            &image('a'),
            CachePlatform::LinuxAmd64,
        )
        .unwrap();
        let other_cache = CacheIdentity::new(
            "repo-1",
            WorkflowCache::parse("cargo-target").unwrap(),
            &image('a'),
            CachePlatform::LinuxAmd64,
        )
        .unwrap();
        let other_image =
            CacheIdentity::new("repo-1", cache, &image('b'), CachePlatform::LinuxAmd64).unwrap();

        assert_eq!(base.digest(), base.digest());
        assert_eq!(base.digest().len(), 64);
        assert_ne!(base.digest(), other_repo.digest());
        assert_ne!(base.digest(), other_cache.digest());
        assert_ne!(base.digest(), other_image.digest());
        assert!(
            CacheIdentity::new(
                " ",
                WorkflowCache::parse("cargo").unwrap(),
                &image('a'),
                CachePlatform::LinuxAmd64,
            )
            .is_err()
        );
    }
}
