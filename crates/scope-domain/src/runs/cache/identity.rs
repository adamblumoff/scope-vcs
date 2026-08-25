use super::definition::WorkflowCache;
use crate::{
    error::DomainError,
    runs::workflow::{definition::WorkflowJobId, identity::WorkflowPath},
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

pub const CACHE_IDENTITY_FORMAT: &str = "scope-cache-v4";

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
