use crate::{GitSegmentManifest, GitStorageError, GitStorageLimits};
use scope_domain::store::{GitHead, GitSegment, SourceBlob};
use scope_object_store::{ContentObjectKind, ObjectStore, ensure_object_size, put_content_object};

#[derive(Debug)]
pub struct GitSnapshotMaterializationError {
    error: GitStorageError,
    orphan_objects: Vec<SourceBlob>,
}

impl GitSnapshotMaterializationError {
    pub fn error(&self) -> &GitStorageError {
        &self.error
    }

    pub fn into_parts(self) -> (GitStorageError, Vec<SourceBlob>) {
        (self.error, self.orphan_objects)
    }

    fn without_orphans(error: GitStorageError) -> Self {
        Self {
            error,
            orphan_objects: Vec::new(),
        }
    }

    fn with_orphan(error: GitStorageError, orphan: SourceBlob) -> Self {
        Self {
            error,
            orphan_objects: vec![orphan],
        }
    }
}

impl std::fmt::Display for GitSnapshotMaterializationError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.error.fmt(formatter)
    }
}

impl std::error::Error for GitSnapshotMaterializationError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(&self.error)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StoredGitSegment {
    pub head: GitHead,
    pub segment: GitSegment,
}

pub fn materialize_incremental_git_segment(
    store: &dyn ObjectStore,
    segment_bytes: &[u8],
    head_oid: String,
    previous: Option<&GitHead>,
    storage_limits: GitStorageLimits,
) -> Result<StoredGitSegment, GitSnapshotMaterializationError> {
    let sequence = storage_limits
        .next_segment_sequence(previous.map(|head| head.segment_sequence))
        .map_err(GitStorageError::from)
        .map_err(GitSnapshotMaterializationError::without_orphans)?;
    materialize_git_segment(
        store,
        segment_bytes,
        head_oid,
        sequence,
        previous.map(|head| head.head_oid.clone()),
        previous.map(|head| head.manifest.clone()),
        previous.map_or(1, |head| head.change_version.saturating_add(1)),
        storage_limits.max_object_bytes(),
    )
}

pub fn materialize_compacted_git_segment(
    store: &dyn ObjectStore,
    segment_bytes: &[u8],
    current_head: &GitHead,
    storage_limits: GitStorageLimits,
) -> Result<StoredGitSegment, GitSnapshotMaterializationError> {
    materialize_git_segment(
        store,
        segment_bytes,
        current_head.head_oid.clone(),
        1,
        None,
        None,
        current_head.change_version,
        storage_limits.max_object_bytes(),
    )
}

#[allow(clippy::too_many_arguments)]
fn materialize_git_segment(
    store: &dyn ObjectStore,
    segment_bytes: &[u8],
    head_oid: String,
    sequence: u64,
    base_oid: Option<String>,
    previous_manifest: Option<SourceBlob>,
    change_version: u64,
    max_object_bytes: usize,
) -> Result<StoredGitSegment, GitSnapshotMaterializationError> {
    ensure_object_size(
        "write",
        "Git segment",
        segment_bytes.len(),
        max_object_bytes,
    )
    .map_err(GitStorageError::from)
    .map_err(GitSnapshotMaterializationError::without_orphans)?;
    let segment = put_content_object(store, ContentObjectKind::GitSegment, segment_bytes)
        .map_err(GitStorageError::from)
        .map_err(GitSnapshotMaterializationError::without_orphans)?;
    let manifest = GitSegmentManifest::new(head_oid.clone(), previous_manifest, segment.clone());
    let manifest_bytes = manifest
        .encode()
        .map_err(|error| GitSnapshotMaterializationError::with_orphan(error, segment.clone()))?;
    ensure_object_size(
        "write",
        "Git segment manifest",
        manifest_bytes.len(),
        max_object_bytes,
    )
    .map_err(GitStorageError::from)
    .map_err(|error| GitSnapshotMaterializationError::with_orphan(error, segment.clone()))?;
    let mut manifest = put_content_object(store, ContentObjectKind::GitManifest, &manifest_bytes)
        .map_err(GitStorageError::from)
        .map_err(|error| GitSnapshotMaterializationError::with_orphan(error, segment.clone()))?;
    manifest.git_oid = head_oid.clone();

    Ok(StoredGitSegment {
        head: GitHead {
            head_oid: head_oid.clone(),
            segment_sequence: sequence,
            change_version,
            manifest: manifest.clone(),
        },
        segment: GitSegment {
            sequence,
            base_oid,
            head_oid,
            object: segment,
            manifest,
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use scope_domain::store::DEFAULT_GIT_FILE_MODE;
    use scope_object_store::MemoryObjectStore;

    fn stored_blob(sha256: &str) -> SourceBlob {
        SourceBlob {
            content_ref: scope_domain::content_ref::ContentRef::git_manifest_sha256(sha256),
            sha256: sha256.to_string(),
            git_oid: "head-1".to_string(),
            git_file_mode: DEFAULT_GIT_FILE_MODE.to_string(),
            size_bytes: 1,
        }
    }

    #[test]
    fn incremental_materialization_owns_chain_metadata_and_content_identity() {
        let store = MemoryObjectStore::new();
        let previous = GitHead {
            head_oid: "head-1".to_string(),
            segment_sequence: 4,
            change_version: 9,
            manifest: stored_blob("previous"),
        };

        let stored = materialize_incremental_git_segment(
            &store,
            b"pack",
            "head-2".to_string(),
            Some(&previous),
            GitStorageLimits::new(4096, 10).unwrap(),
        )
        .unwrap();

        assert_eq!(stored.head.segment_sequence, 5);
        assert_eq!(stored.head.change_version, 10);
        assert_eq!(stored.segment.base_oid.as_deref(), Some("head-1"));
        assert_eq!(stored.segment.head_oid, "head-2");
        assert!(matches!(
            stored.segment.object.content_ref,
            scope_domain::content_ref::ContentRef::GitSegmentSha256(_)
        ));
        assert!(matches!(
            stored.head.manifest.content_ref,
            scope_domain::content_ref::ContentRef::GitManifestSha256(_)
        ));
        assert_eq!(stored.head.manifest, stored.segment.manifest);
    }

    #[test]
    fn compacted_materialization_preserves_visible_version_and_resets_chain() {
        let store = MemoryObjectStore::new();
        let current = GitHead {
            head_oid: "head".to_string(),
            segment_sequence: 7,
            change_version: 11,
            manifest: stored_blob("current"),
        };

        let stored = materialize_compacted_git_segment(
            &store,
            b"pack",
            &current,
            GitStorageLimits::new(4096, 10).unwrap(),
        )
        .unwrap();

        assert_eq!(stored.head.segment_sequence, 1);
        assert_eq!(stored.head.change_version, 11);
        assert_eq!(stored.segment.sequence, 1);
        assert_eq!(stored.segment.base_oid, None);
        assert_eq!(stored.segment.head_oid, "head");
    }

    #[test]
    fn manifest_failure_reports_the_stored_segment_for_cleanup() {
        let store = MemoryObjectStore::new();

        let failure = materialize_incremental_git_segment(
            &store,
            b"x",
            "head".to_string(),
            None,
            GitStorageLimits::new(1, 10).unwrap(),
        )
        .unwrap_err();
        let (_, orphans) = failure.into_parts();

        assert_eq!(orphans.len(), 1);
        assert!(matches!(
            orphans[0].content_ref,
            scope_domain::content_ref::ContentRef::GitSegmentSha256(_)
        ));
    }
}
