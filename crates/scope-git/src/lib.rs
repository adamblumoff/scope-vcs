mod projection_identity;
#[cfg(feature = "storage")]
mod snapshot;
mod tree_path;

pub use projection_identity::{
    PROJECTION_IDENTITY_VERSION, ProjectionIdentityError, projection_head_oid,
};
#[cfg(feature = "storage")]
pub use snapshot::{
    GitSnapshotMaterializationError, PreparedGitPush, StoredGitPush, materialize_git_push,
    prepare_compacted_git_pack, prepare_git_push, store_compacted_git_pack,
};
pub use tree_path::{GitTreePath, GitTreePathError};

use scope_domain::content::SourceBlob;
use scope_domain::content_ref::ContentRef;
#[cfg(feature = "storage")]
use scope_object_store::ObjectStoreError;
use serde::{Deserialize, Serialize};
use thiserror::Error;

pub const DEFAULT_GIT_BRANCH: &str = "main";
pub const GIT_SNAPSHOT_MANIFEST_VERSION: u8 = 2;
pub const DEFAULT_GIT_COMPACTION_SPANS: usize = 32;
pub const DEFAULT_GIT_STORAGE_MAX_OBJECT_BYTES: usize = 128 * 1024 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GitStorageLimits {
    max_object_bytes: usize,
}

impl GitStorageLimits {
    pub fn new(max_object_bytes: usize) -> Result<Self, GitStorageLimitError> {
        if max_object_bytes == 0 {
            return Err(GitStorageLimitError::ZeroObjectBytes);
        }
        Ok(Self { max_object_bytes })
    }

    pub fn max_object_bytes(self) -> usize {
        self.max_object_bytes
    }

    pub fn next_push_sequence(
        self,
        previous_sequence: Option<u64>,
    ) -> Result<u64, GitStorageLimitError> {
        previous_sequence
            .unwrap_or(0)
            .checked_add(1)
            .ok_or(GitStorageLimitError::SequenceOverflow)
    }
}

impl Default for GitStorageLimits {
    fn default() -> Self {
        Self {
            max_object_bytes: DEFAULT_GIT_STORAGE_MAX_OBJECT_BYTES,
        }
    }
}

#[derive(Clone, Debug, Error, PartialEq, Eq)]
pub enum GitStorageLimitError {
    #[error("Git object size limit must be greater than zero")]
    ZeroObjectBytes,
    #[error("Git push sequence overflow")]
    SequenceOverflow,
}

#[derive(Debug, Error)]
pub enum GitStorageError {
    #[error(transparent)]
    StorageLimit(#[from] GitStorageLimitError),
    #[cfg(feature = "storage")]
    #[error(transparent)]
    ObjectStore(#[from] ObjectStoreError),
    #[error("failed to encode Git snapshot manifest: {0}")]
    ManifestEncode(#[source] serde_json::Error),
    #[error("failed to decode Git snapshot manifest: {0}")]
    ManifestDecode(#[source] serde_json::Error),
    #[error("unsupported Git snapshot manifest version {version}")]
    UnsupportedManifestVersion { version: u8 },
    #[error("Git blob reference requires a manifest")]
    ManifestReferenceRequired,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct GitSnapshotManifest {
    pub version: u8,
    pub head_oid: String,
    pub push_sequence: u64,
}

impl GitSnapshotManifest {
    pub fn new(head_oid: String, push_sequence: u64) -> Self {
        Self {
            version: GIT_SNAPSHOT_MANIFEST_VERSION,
            head_oid,
            push_sequence,
        }
    }

    pub fn encode(&self) -> Result<Vec<u8>, GitStorageError> {
        serde_json::to_vec(self).map_err(GitStorageError::ManifestEncode)
    }

    pub fn decode(bytes: &[u8]) -> Result<Self, GitStorageError> {
        let manifest: Self =
            serde_json::from_slice(bytes).map_err(GitStorageError::ManifestDecode)?;
        if manifest.version != GIT_SNAPSHOT_MANIFEST_VERSION {
            return Err(GitStorageError::UnsupportedManifestVersion {
                version: manifest.version,
            });
        }
        Ok(manifest)
    }
}

pub fn is_git_snapshot_manifest(snapshot: &SourceBlob) -> bool {
    matches!(snapshot.content_ref, ContentRef::GitManifestSha256(_))
}

pub fn git_blob_reference(
    manifest: &SourceBlob,
    oid: String,
    mode: String,
    size_bytes: u64,
) -> Result<SourceBlob, GitStorageError> {
    let ContentRef::GitManifestSha256(_) = &manifest.content_ref else {
        return Err(GitStorageError::ManifestReferenceRequired);
    };
    Ok(SourceBlob {
        content_ref: ContentRef::git_blob(oid.clone()),
        sha256: oid.clone(),
        git_oid: oid,
        git_file_mode: mode,
        size_bytes,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use scope_domain::content::DEFAULT_GIT_FILE_MODE;

    fn manifest(sha256: &str) -> SourceBlob {
        SourceBlob {
            content_ref: ContentRef::git_manifest_sha256(sha256),
            sha256: sha256.to_string(),
            git_oid: "head".to_string(),
            git_file_mode: DEFAULT_GIT_FILE_MODE.to_string(),
            size_bytes: 10,
        }
    }

    #[test]
    fn git_blob_identity_does_not_depend_on_manifest() {
        let old = git_blob_reference(
            &manifest("old-sha"),
            "blob-oid".to_string(),
            DEFAULT_GIT_FILE_MODE.to_string(),
            42,
        )
        .unwrap();
        let new = git_blob_reference(
            &manifest("new-sha"),
            "blob-oid".to_string(),
            DEFAULT_GIT_FILE_MODE.to_string(),
            42,
        )
        .unwrap();

        assert_eq!(old.content_ref, ContentRef::git_blob("blob-oid"));
        assert_eq!(new.content_ref, old.content_ref);
    }

    #[test]
    fn storage_limits_do_not_bound_logical_sequence() {
        let limits = GitStorageLimits::new(4).unwrap();

        assert_eq!(limits.next_push_sequence(None).unwrap(), 1);
        assert_eq!(limits.next_push_sequence(Some(2)).unwrap(), 3);
    }

    #[test]
    fn storage_limits_reject_zero_values() {
        assert_eq!(
            GitStorageLimits::new(0).unwrap_err(),
            GitStorageLimitError::ZeroObjectBytes
        );
    }

    #[test]
    fn default_storage_limits_match_the_shared_policy() {
        let limits = GitStorageLimits::default();

        assert_eq!(
            limits.max_object_bytes(),
            DEFAULT_GIT_STORAGE_MAX_OBJECT_BYTES
        );
    }

    #[test]
    fn snapshot_manifest_contains_no_physical_pack_topology() {
        let manifest = GitSnapshotManifest::new("head".to_string(), 42);
        let encoded = manifest.encode().unwrap();
        let decoded = GitSnapshotManifest::decode(&encoded).unwrap();

        assert_eq!(decoded.head_oid, "head");
        assert_eq!(decoded.push_sequence, 42);
        assert!(!String::from_utf8(encoded).unwrap().contains("segment"));
    }

    #[test]
    fn snapshot_manifests_reject_unsupported_versions() {
        let encoded = br#"{"version":1,"head_oid":"head","push_sequence":1}"#;

        assert!(matches!(
            GitSnapshotManifest::decode(encoded),
            Err(GitStorageError::UnsupportedManifestVersion { version: 1 })
        ));
    }
}
