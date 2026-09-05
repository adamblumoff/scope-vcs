use super::ObjectStore;
use sha1::{Digest as _, Sha1};
use sha2::Sha256;
use {
    crate::ObjectStoreError,
    scope_domain::{
        content::{DEFAULT_GIT_FILE_MODE, SourceBlob},
        content_ref::ContentRef,
    },
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ContentObjectKind {
    Blob,
    GitBundle,
    GitManifest,
}

impl ContentObjectKind {
    fn content_ref(self, sha256: String) -> ContentRef {
        match self {
            Self::Blob => ContentRef::blob_sha256(sha256),
            Self::GitBundle => ContentRef::git_bundle_sha256(sha256),
            Self::GitManifest => ContentRef::git_manifest_sha256(sha256),
        }
    }
}

pub fn object_key(blob: &SourceBlob) -> String {
    object_key_for_content_ref(&blob.content_ref)
}

pub fn object_key_for_content_ref(content_ref: &ContentRef) -> String {
    match content_ref {
        ContentRef::BlobSha256(sha256) => format!("objects/blobs/{sha256}"),
        ContentRef::GitBundleSha256(sha256) => format!("objects/git-bundles/{sha256}"),
        ContentRef::GitManifestSha256(sha256) => format!("objects/git-manifests/{sha256}"),
        ContentRef::GitBlob { git_oid } => format!("git-blobs/{git_oid}"),
    }
}

pub fn content_object_for_bytes(kind: ContentObjectKind, bytes: &[u8]) -> SourceBlob {
    let sha256 = hex::encode(Sha256::digest(bytes));
    SourceBlob {
        content_ref: kind.content_ref(sha256.clone()),
        sha256,
        git_oid: git_blob_oid(bytes),
        git_file_mode: DEFAULT_GIT_FILE_MODE.to_string(),
        size_bytes: bytes.len() as u64,
    }
}

pub fn put_source_blob(
    store: &dyn ObjectStore,
    bytes: &[u8],
) -> Result<SourceBlob, ObjectStoreError> {
    put_content_object(store, ContentObjectKind::Blob, bytes.to_vec())
}

pub fn put_content_object(
    store: &dyn ObjectStore,
    kind: ContentObjectKind,
    bytes: Vec<u8>,
) -> Result<SourceBlob, ObjectStoreError> {
    let blob = content_object_for_bytes(kind, &bytes);
    store.put(&object_key(&blob), bytes)?;
    Ok(blob)
}

pub fn source_blob_bytes(
    store: &dyn ObjectStore,
    blob: &SourceBlob,
) -> Result<Vec<u8>, ObjectStoreError> {
    let key = object_key(blob);
    verified_source_blob_bytes(blob, store.get(&key)?, &key)
}

pub fn source_blob_bytes_bounded(
    store: &dyn ObjectStore,
    blob: &SourceBlob,
    max_bytes: usize,
) -> Result<Vec<u8>, ObjectStoreError> {
    let key = object_key(blob);
    verified_source_blob_bytes(blob, store.get_bounded(&key, max_bytes)?, &key)
}

fn verified_source_blob_bytes(
    blob: &SourceBlob,
    bytes: Vec<u8>,
    key: &str,
) -> Result<Vec<u8>, ObjectStoreError> {
    let sha256 = hex::encode(Sha256::digest(&bytes));
    if sha256 != blob.sha256 {
        return Err(ObjectStoreError::integrity(format!(
            "object {key} failed sha256 verification"
        )));
    }
    Ok(bytes)
}

pub fn delete_source_blobs<'a>(
    store: &dyn ObjectStore,
    blobs: impl IntoIterator<Item = &'a SourceBlob>,
) -> Result<(), ObjectStoreError> {
    let mut keys = blobs.into_iter().map(object_key).collect::<Vec<_>>();
    keys.sort_unstable();
    keys.dedup();
    for key in keys {
        store.delete(&key)?;
    }
    Ok(())
}

fn git_blob_oid(bytes: &[u8]) -> String {
    let mut hasher = Sha1::new();
    hasher.update(format!("blob {}\0", bytes.len()).as_bytes());
    hasher.update(bytes);
    hex::encode(hasher.finalize())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{MemoryObjectStore, ObjectStoreErrorKind};

    #[test]
    fn source_blob_round_trip_preserves_content_metadata() {
        let store = MemoryObjectStore::new();
        let bytes = b"hello from scope";

        let blob = put_source_blob(&store, bytes).unwrap();

        assert_eq!(object_key(&blob), format!("objects/blobs/{}", blob.sha256));
        assert_eq!(blob.size_bytes, bytes.len() as u64);
        assert_eq!(blob.git_file_mode, DEFAULT_GIT_FILE_MODE);
        assert_eq!(source_blob_bytes(&store, &blob).unwrap(), bytes);
    }

    #[test]
    fn source_blob_reads_reject_content_that_fails_sha256_verification() {
        let store = MemoryObjectStore::new();
        let blob = content_object_for_bytes(ContentObjectKind::Blob, b"expected");
        let key = object_key(&blob);
        store.put(&key, b"different".to_vec()).unwrap();

        let error = source_blob_bytes(&store, &blob).unwrap_err();

        assert_eq!(error.kind, ObjectStoreErrorKind::Integrity);
        assert_eq!(
            error.message,
            format!("object {key} failed sha256 verification")
        );
    }

    #[test]
    fn bounded_source_blob_reads_preserve_payload_too_large_classification() {
        let store = MemoryObjectStore::new();
        let blob = put_source_blob(&store, b"five!").unwrap();
        let key = object_key(&blob);

        let error = source_blob_bytes_bounded(&store, &blob, 4).unwrap_err();

        assert_eq!(error.kind, ObjectStoreErrorKind::PayloadTooLarge);
        assert_eq!(
            error.message,
            format!("object store read for {key} is too large: 5 bytes exceeds 4 bytes")
        );
    }
}
