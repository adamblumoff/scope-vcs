use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};

// The format covers the store layout, metadata records, and Docker volume identity.
// Old ephemeral cache state is refused rather than kept through a second reader.
pub(super) const CACHE_FORMAT: u8 = 5;
const RUNNER_NAMESPACE_HEX_LENGTH: usize = 32;
const VOLUME_DIGEST_HEX_LENGTH: usize = 40;

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct CacheLocation {
    pub(super) runner_namespace: String,
    pub(super) identity_digest: String,
    pub(super) volume_name: String,
    pub(super) record_path: PathBuf,
    pub(super) backing_path: PathBuf,
}

impl CacheLocation {
    pub(super) fn for_runner(root: &Path, runner_id: &str, identity_digest: &str) -> Self {
        Self::from_namespace(root, runner_namespace(runner_id), identity_digest)
    }

    pub(super) fn from_namespace(
        root: &Path,
        runner_namespace: String,
        identity_digest: &str,
    ) -> Self {
        Self {
            volume_name: volume_name(&runner_namespace, identity_digest),
            record_path: root
                .join("metadata")
                .join(&runner_namespace)
                .join(format!("{identity_digest}.json")),
            backing_path: root
                .join("data")
                .join(&runner_namespace)
                .join(identity_digest),
            runner_namespace,
            identity_digest: identity_digest.to_string(),
        }
    }
}

pub(super) fn runner_namespace(runner_id: &str) -> String {
    let digest = hex::encode(Sha256::digest(runner_id.as_bytes()));
    digest[..RUNNER_NAMESPACE_HEX_LENGTH].to_string()
}

pub(super) fn volume_name(runner_namespace: &str, identity_digest: &str) -> String {
    let mut physical_identity = Sha256::new();
    physical_identity.update(runner_namespace.as_bytes());
    physical_identity.update([0]);
    physical_identity.update(identity_digest.as_bytes());
    let digest = hex::encode(physical_identity.finalize());
    format!(
        "scope-cache-v{CACHE_FORMAT}-{}",
        &digest[..VOLUME_DIGEST_HEX_LENGTH]
    )
}
