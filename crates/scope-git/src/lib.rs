mod projection_identity;
#[cfg(feature = "storage")]
mod snapshot;

pub use projection_identity::{
    PROJECTION_IDENTITY_VERSION, ProjectionIdentityError, projection_head_oid,
};
#[cfg(feature = "storage")]
pub use snapshot::{
    GitSnapshotMaterializationError, StoredGitSegment, materialize_compacted_git_segment,
    materialize_incremental_git_segment,
};

use scope_domain::content_ref::ContentRef;
use scope_domain::store::{GitHead, GitSegment, SourceBlob};
#[cfg(feature = "storage")]
use scope_object_store::ObjectStoreError;
use serde::{Deserialize, Serialize};
use thiserror::Error;

pub const DEFAULT_GIT_BRANCH: &str = "main";

pub const GIT_SEGMENT_MANIFEST_VERSION: u8 = 1;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GitStorageLimits {
    max_object_bytes: usize,
    max_chain_depth: usize,
}

impl GitStorageLimits {
    pub fn new(
        max_object_bytes: usize,
        max_chain_depth: usize,
    ) -> Result<Self, GitStorageLimitError> {
        if max_object_bytes == 0 {
            return Err(GitStorageLimitError::ZeroObjectBytes);
        }
        if max_chain_depth == 0 {
            return Err(GitStorageLimitError::ZeroChainDepth);
        }
        Ok(Self {
            max_object_bytes,
            max_chain_depth,
        })
    }

    pub fn max_object_bytes(self) -> usize {
        self.max_object_bytes
    }

    pub fn max_chain_depth(self) -> usize {
        self.max_chain_depth
    }

    pub fn next_segment_sequence(
        self,
        previous_sequence: Option<u64>,
    ) -> Result<u64, GitStorageLimitError> {
        let previous_sequence = previous_sequence.unwrap_or(0);
        let previous_depth = usize::try_from(previous_sequence)
            .map_err(|_| GitStorageLimitError::SequenceOverflow)?;
        if previous_depth >= self.max_chain_depth {
            return Err(GitStorageLimitError::ChainDepthReached {
                max_chain_depth: self.max_chain_depth,
            });
        }
        previous_sequence
            .checked_add(1)
            .ok_or(GitStorageLimitError::SequenceOverflow)
    }
}

#[derive(Clone, Debug, Error, PartialEq, Eq)]
pub enum GitStorageLimitError {
    #[error("Git object size limit must be greater than zero")]
    ZeroObjectBytes,
    #[error("Git segment chain depth limit must be greater than zero")]
    ZeroChainDepth,
    #[error("Git segment chain has reached maximum depth of {max_chain_depth}")]
    ChainDepthReached { max_chain_depth: usize },
    #[error("Git segment sequence overflow")]
    SequenceOverflow,
}

#[derive(Clone, Debug, Error, PartialEq, Eq)]
pub enum GitCompactionContractError {
    #[error("Git compaction replacement must contain exactly one segment")]
    InvalidSequence,
    #[error("Git compaction replacement segment must not have a base")]
    UnexpectedBase,
    #[error("Git compaction replacement must preserve the visible head")]
    HeadMismatch,
    #[error("Git compaction replacement head and segment must share a manifest")]
    ManifestMismatch,
}

pub fn validate_compacted_replacement(
    head: &GitHead,
    segment: &GitSegment,
) -> Result<(), GitCompactionContractError> {
    if head.segment_sequence != 1 || segment.sequence != 1 {
        return Err(GitCompactionContractError::InvalidSequence);
    }
    if segment.base_oid.is_some() {
        return Err(GitCompactionContractError::UnexpectedBase);
    }
    if head.head_oid != segment.head_oid {
        return Err(GitCompactionContractError::HeadMismatch);
    }
    if head.manifest != segment.manifest {
        return Err(GitCompactionContractError::ManifestMismatch);
    }
    Ok(())
}

#[derive(Debug, Error)]
pub enum GitStorageError {
    #[error(transparent)]
    StorageLimit(#[from] GitStorageLimitError),
    #[cfg(feature = "storage")]
    #[error(transparent)]
    ObjectStore(#[from] ObjectStoreError),
    #[error("failed to encode Git segment manifest: {0}")]
    ManifestEncode(#[source] serde_json::Error),
    #[error("failed to decode Git segment manifest: {0}")]
    ManifestDecode(#[source] serde_json::Error),
    #[error("unsupported Git segment manifest version {version}")]
    UnsupportedManifestVersion { version: u8 },
    #[error("Git blob reference requires a manifest")]
    ManifestReferenceRequired,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct GitSegmentManifest {
    pub version: u8,
    pub head_oid: String,
    pub previous: Option<SourceBlob>,
    pub segment: SourceBlob,
}

impl GitSegmentManifest {
    pub fn new(head_oid: String, previous: Option<SourceBlob>, segment: SourceBlob) -> Self {
        Self {
            version: GIT_SEGMENT_MANIFEST_VERSION,
            head_oid,
            previous,
            segment,
        }
    }

    pub fn encode(&self) -> Result<Vec<u8>, GitStorageError> {
        serde_json::to_vec(self).map_err(GitStorageError::ManifestEncode)
    }

    pub fn decode(bytes: &[u8]) -> Result<Self, GitStorageError> {
        let manifest: Self =
            serde_json::from_slice(bytes).map_err(GitStorageError::ManifestDecode)?;
        if manifest.version != GIT_SEGMENT_MANIFEST_VERSION {
            return Err(GitStorageError::UnsupportedManifestVersion {
                version: manifest.version,
            });
        }
        Ok(manifest)
    }
}

pub fn is_git_segment_manifest(snapshot: &SourceBlob) -> bool {
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
    use scope_domain::store::DEFAULT_GIT_FILE_MODE;

    fn manifest(_id: &str, sha256: &str) -> SourceBlob {
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
            &manifest("old", "old-sha"),
            "blob-oid".to_string(),
            DEFAULT_GIT_FILE_MODE.to_string(),
            42,
        )
        .unwrap();
        let new = git_blob_reference(
            &manifest("new", "new-sha"),
            "blob-oid".to_string(),
            DEFAULT_GIT_FILE_MODE.to_string(),
            42,
        )
        .unwrap();

        assert_eq!(old.content_ref, ContentRef::git_blob("blob-oid"));
        assert_eq!(new.content_ref, old.content_ref);
    }

    #[test]
    fn storage_limits_accept_exact_chain_boundary_and_reject_the_next_segment() {
        let limits = GitStorageLimits::new(4, 2).unwrap();

        assert_eq!(limits.next_segment_sequence(None).unwrap(), 1);
        assert_eq!(limits.next_segment_sequence(Some(1)).unwrap(), 2);
        assert_eq!(
            limits.next_segment_sequence(Some(2)).unwrap_err(),
            GitStorageLimitError::ChainDepthReached { max_chain_depth: 2 }
        );
    }

    #[test]
    fn storage_limits_reject_zero_values() {
        assert_eq!(
            GitStorageLimits::new(0, 1).unwrap_err(),
            GitStorageLimitError::ZeroObjectBytes
        );
        assert_eq!(
            GitStorageLimits::new(1, 0).unwrap_err(),
            GitStorageLimitError::ZeroChainDepth
        );
    }

    #[test]
    fn compacted_replacement_contract_requires_one_root_segment_for_the_same_head() {
        let new_manifest = manifest("new", "new-sha");
        let head = GitHead {
            head_oid: "head".to_string(),
            segment_sequence: 1,
            change_version: 2,
            manifest: new_manifest.clone(),
        };
        let segment = GitSegment {
            sequence: 1,
            base_oid: None,
            head_oid: "head".to_string(),
            object: manifest("segment", "segment-sha"),
            manifest: new_manifest,
        };

        validate_compacted_replacement(&head, &segment).unwrap();

        let mut invalid = segment.clone();
        invalid.base_oid = Some("base".to_string());
        assert_eq!(
            validate_compacted_replacement(&head, &invalid).unwrap_err(),
            GitCompactionContractError::UnexpectedBase
        );
    }

    #[test]
    fn manifests_reject_unsupported_versions() {
        let encoded = br#"{"version":2,"head_oid":"head","previous":null,"segment":{"content_ref":{"GitSegmentSha256":"sha"},"sha256":"sha","git_oid":"head","git_file_mode":"100644","size_bytes":1}}"#;

        assert!(matches!(
            GitSegmentManifest::decode(encoded),
            Err(GitStorageError::UnsupportedManifestVersion { version: 2 })
        ));
    }
}
