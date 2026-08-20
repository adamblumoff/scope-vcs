use crate::{GitSnapshotManifest, GitStorageError, GitStorageLimits};
use scope_domain::store::{GitHead, GitPackSpan, SourceBlob};
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
pub struct StoredGitPush {
    pub head: GitHead,
    pub pack_span: GitPackSpan,
}

pub fn materialize_git_push(
    store: &dyn ObjectStore,
    pack_bytes: &[u8],
    head_oid: String,
    previous: Option<&GitHead>,
    storage_limits: GitStorageLimits,
) -> Result<StoredGitPush, GitSnapshotMaterializationError> {
    let sequence = storage_limits
        .next_push_sequence(previous.map(|head| head.push_sequence))
        .map_err(GitStorageError::from)
        .map_err(GitSnapshotMaterializationError::without_orphans)?;
    materialize_git_push_objects(
        store,
        pack_bytes,
        head_oid,
        sequence,
        previous.map(|head| head.head_oid.clone()),
        previous.map_or(1, |head| head.change_version.saturating_add(1)),
        storage_limits.max_object_bytes(),
    )
}

pub fn store_compacted_git_pack(
    store: &dyn ObjectStore,
    pack_bytes: &[u8],
    storage_limits: GitStorageLimits,
) -> Result<SourceBlob, GitSnapshotMaterializationError> {
    ensure_object_size(
        "write",
        "compacted Git pack",
        pack_bytes.len(),
        storage_limits.max_object_bytes(),
    )
    .map_err(GitStorageError::from)
    .map_err(GitSnapshotMaterializationError::without_orphans)?;
    put_content_object(store, ContentObjectKind::GitSegment, pack_bytes)
        .map_err(GitStorageError::from)
        .map_err(GitSnapshotMaterializationError::without_orphans)
}

#[allow(clippy::too_many_arguments)]
fn materialize_git_push_objects(
    store: &dyn ObjectStore,
    pack_bytes: &[u8],
    head_oid: String,
    sequence: u64,
    base_oid: Option<String>,
    change_version: u64,
    max_object_bytes: usize,
) -> Result<StoredGitPush, GitSnapshotMaterializationError> {
    ensure_object_size("write", "Git pack", pack_bytes.len(), max_object_bytes)
        .map_err(GitStorageError::from)
        .map_err(GitSnapshotMaterializationError::without_orphans)?;
    let pack = put_content_object(store, ContentObjectKind::GitSegment, pack_bytes)
        .map_err(GitStorageError::from)
        .map_err(GitSnapshotMaterializationError::without_orphans)?;
    let manifest = GitSnapshotManifest::new(head_oid.clone(), sequence);
    let manifest_bytes = manifest
        .encode()
        .map_err(|error| GitSnapshotMaterializationError::with_orphan(error, pack.clone()))?;
    ensure_object_size(
        "write",
        "Git snapshot manifest",
        manifest_bytes.len(),
        max_object_bytes,
    )
    .map_err(GitStorageError::from)
    .map_err(|error| GitSnapshotMaterializationError::with_orphan(error, pack.clone()))?;
    let mut manifest = put_content_object(store, ContentObjectKind::GitManifest, &manifest_bytes)
        .map_err(GitStorageError::from)
        .map_err(|error| GitSnapshotMaterializationError::with_orphan(error, pack.clone()))?;
    manifest.git_oid = head_oid.clone();

    Ok(StoredGitPush {
        head: GitHead {
            head_oid: head_oid.clone(),
            push_sequence: sequence,
            change_version,
            manifest,
        },
        pack_span: GitPackSpan {
            first_sequence: sequence,
            last_sequence: sequence,
            geometric_tier: 0,
            base_oid,
            head_oid,
            object: pack,
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
    fn push_materialization_separates_snapshot_identity_from_pack_layout() {
        let store = MemoryObjectStore::new();
        let previous = GitHead {
            head_oid: "head-1".to_string(),
            push_sequence: 4,
            change_version: 9,
            manifest: stored_blob("previous"),
        };

        let stored = materialize_git_push(
            &store,
            b"pack",
            "head-2".to_string(),
            Some(&previous),
            GitStorageLimits::new(4096, 10).unwrap(),
        )
        .unwrap();

        assert_eq!(stored.head.push_sequence, 5);
        assert_eq!(stored.head.change_version, 10);
        assert_eq!(stored.pack_span.first_sequence, 5);
        assert_eq!(stored.pack_span.last_sequence, 5);
        assert_eq!(stored.pack_span.base_oid.as_deref(), Some("head-1"));
        assert_eq!(stored.pack_span.head_oid, "head-2");
        assert!(matches!(
            stored.pack_span.object.content_ref,
            scope_domain::content_ref::ContentRef::GitSegmentSha256(_)
        ));
        let manifest_bytes = store
            .get(&scope_object_store::object_key(&stored.head.manifest))
            .unwrap();
        let manifest = GitSnapshotManifest::decode(&manifest_bytes).unwrap();
        assert_eq!(manifest.push_sequence, 5);
        assert_eq!(manifest.head_oid, "head-2");
    }

    #[test]
    fn compacted_pack_storage_does_not_create_a_snapshot_manifest() {
        let store = MemoryObjectStore::new();
        let stored =
            store_compacted_git_pack(&store, b"pack", GitStorageLimits::new(4096, 10).unwrap())
                .unwrap();

        assert!(matches!(
            stored.content_ref,
            scope_domain::content_ref::ContentRef::GitSegmentSha256(_)
        ));
    }

    #[test]
    fn manifest_failure_reports_the_stored_pack_for_cleanup() {
        let store = MemoryObjectStore::new();
        let failure = materialize_git_push(
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
