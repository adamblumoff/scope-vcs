use serde::{Deserialize, Serialize};

/// Stable semantic identity for stored content.
///
/// Variants describe how content participates in the Git/content model; adapters
/// remain responsible for turning this identity into a physical storage location.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum ContentRef {
    BlobSha256(String),
    GitBundleSha256(String),
    GitSegmentSha256(String),
    GitManifestSha256(String),
    GitBlob { git_oid: String },
}

impl ContentRef {
    pub fn blob_sha256(sha256: impl Into<String>) -> Self {
        Self::BlobSha256(sha256.into())
    }

    pub fn git_bundle_sha256(sha256: impl Into<String>) -> Self {
        Self::GitBundleSha256(sha256.into())
    }

    pub fn git_segment_sha256(sha256: impl Into<String>) -> Self {
        Self::GitSegmentSha256(sha256.into())
    }

    pub fn git_manifest_sha256(sha256: impl Into<String>) -> Self {
        Self::GitManifestSha256(sha256.into())
    }

    pub fn git_blob(git_oid: impl Into<String>) -> Self {
        Self::GitBlob {
            git_oid: git_oid.into(),
        }
    }

    pub fn sha256(&self) -> Option<&str> {
        match self {
            Self::BlobSha256(sha256)
            | Self::GitBundleSha256(sha256)
            | Self::GitSegmentSha256(sha256)
            | Self::GitManifestSha256(sha256) => Some(sha256),
            Self::GitBlob { .. } => None,
        }
    }
}
