use crate::{GitSnapshotManifest, GitStorageError, GitStorageLimits};
use scope_domain::{
    content::SourceBlob,
    repository::git::{GitHead, GitPackSpan, GitSegmentRef},
};
use scope_object_store::{
    ContentObjectKind, ObjectStore, content_object_for_bytes, ensure_object_size, object_key,
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StoredGitPush {
    pub head: GitHead,
    pub pack_span: GitPackSpan,
}

/// Owns only the small snapshot manifest. The Git segment has already been
/// staged by the dedicated segment store and is represented by immutable
/// metadata, never by another full-pack allocation.
#[derive(Debug)]
pub struct PreparedGitPush {
    stored: StoredGitPush,
    manifest_bytes: Vec<u8>,
}

impl PreparedGitPush {
    pub fn manifest(&self) -> &SourceBlob {
        &self.stored.head.manifest
    }

    pub fn store_manifest(self, store: &dyn ObjectStore) -> Result<StoredGitPush, GitStorageError> {
        store.put(
            &object_key(&self.stored.head.manifest),
            &self.manifest_bytes,
        )?;
        Ok(self.stored)
    }
}

pub fn prepare_git_push(
    segment: GitSegmentRef,
    head_oid: String,
    previous: Option<&GitHead>,
    storage_limits: GitStorageLimits,
) -> Result<PreparedGitPush, GitStorageError> {
    ensure_object_size(
        "write",
        "Git pack",
        usize::try_from(segment.plaintext_bytes).unwrap_or(usize::MAX),
        storage_limits.max_object_bytes(),
    )?;
    let sequence = storage_limits.next_push_sequence(previous.map(|head| head.push_sequence))?;
    prepare_git_push_objects(
        segment,
        head_oid,
        sequence,
        previous.map(|head| head.head_oid.clone()),
        previous.map_or(1, |head| head.change_version.saturating_add(1)),
        storage_limits.max_object_bytes(),
    )
}

fn prepare_git_push_objects(
    segment: GitSegmentRef,
    head_oid: String,
    sequence: u64,
    base_oid: Option<String>,
    change_version: u64,
    max_object_bytes: usize,
) -> Result<PreparedGitPush, GitStorageError> {
    let manifest = GitSnapshotManifest::new(head_oid.clone(), sequence);
    let manifest_bytes = manifest.encode()?;
    ensure_object_size(
        "write",
        "Git snapshot manifest",
        manifest_bytes.len(),
        max_object_bytes,
    )?;
    let mut manifest = content_object_for_bytes(ContentObjectKind::GitManifest, &manifest_bytes);
    manifest.git_oid = head_oid.clone();

    Ok(PreparedGitPush {
        stored: StoredGitPush {
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
                segment,
            },
        },
        manifest_bytes,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use scope_domain::content::DEFAULT_GIT_FILE_MODE;
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

    fn segment() -> GitSegmentRef {
        GitSegmentRef {
            segment_id: "segment-5".to_string(),
            sha256: "pack-sha".to_string(),
            plaintext_bytes: 4,
            encoding_version: 2,
        }
    }

    #[test]
    fn push_preparation_separates_snapshot_identity_from_pack_layout() {
        let store = MemoryObjectStore::new();
        let previous = GitHead {
            head_oid: "head-1".to_string(),
            push_sequence: 4,
            change_version: 9,
            manifest: stored_blob("previous"),
        };

        let stored = prepare_git_push(
            segment(),
            "head-2".to_string(),
            Some(&previous),
            GitStorageLimits::new(4096).unwrap(),
        )
        .unwrap()
        .store_manifest(&store)
        .unwrap();

        assert_eq!(stored.head.push_sequence, 5);
        assert_eq!(stored.head.change_version, 10);
        assert_eq!(stored.pack_span.first_sequence, 5);
        assert_eq!(stored.pack_span.last_sequence, 5);
        assert_eq!(stored.pack_span.base_oid.as_deref(), Some("head-1"));
        assert_eq!(stored.pack_span.head_oid, "head-2");
        assert_eq!(stored.pack_span.segment, segment());
        let manifest_bytes = store
            .get(&scope_object_store::object_key(&stored.head.manifest))
            .unwrap();
        let manifest = GitSnapshotManifest::decode(&manifest_bytes).unwrap();
        assert_eq!(manifest.push_sequence, 5);
        assert_eq!(manifest.head_oid, "head-2");
    }

    #[test]
    fn preparation_rejects_segment_larger_than_storage_limit() {
        let error = prepare_git_push(
            segment(),
            "head".to_string(),
            None,
            GitStorageLimits::new(3).unwrap(),
        )
        .unwrap_err();

        assert!(error.to_string().contains("exceeds 3 bytes"));
    }
}
