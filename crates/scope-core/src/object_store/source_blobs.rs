use super::ObjectStore;
use crate::{
    domain::store::{DEFAULT_GIT_FILE_MODE, SourceBlob},
    error::ApiError,
};
use sha1::{Digest as _, Sha1};
use sha2::Sha256;

pub fn repo_object_for_bytes(kind: &str, object_id: &str, bytes: &[u8]) -> SourceBlob {
    let sha256 = hex::encode(Sha256::digest(bytes));
    let git_oid = git_blob_oid(bytes);
    SourceBlob {
        object_key: format!("objects/{kind}/{object_id}"),
        sha256,
        git_oid,
        git_file_mode: DEFAULT_GIT_FILE_MODE.to_string(),
        size_bytes: bytes.len() as u64,
    }
}

pub fn put_source_blob(
    store: &dyn ObjectStore,
    _repo_id: &str,
    bytes: &[u8],
) -> Result<SourceBlob, ApiError> {
    put_repo_object(store, _repo_id, "blobs", bytes)
}

pub fn put_repo_object(
    store: &dyn ObjectStore,
    _repo_id: &str,
    kind: &str,
    bytes: &[u8],
) -> Result<SourceBlob, ApiError> {
    let object_id = random_object_id()?;
    let blob = repo_object_for_bytes(kind, &object_id, bytes);
    store.put(&blob.object_key, bytes)?;
    Ok(blob)
}

pub fn source_blob_bytes(store: &dyn ObjectStore, blob: &SourceBlob) -> Result<Vec<u8>, ApiError> {
    let bytes = store.get(&blob.object_key)?;
    let sha256 = hex::encode(Sha256::digest(&bytes));
    if sha256 != blob.sha256 {
        return Err(ApiError::internal_message(format!(
            "object {} failed sha256 verification",
            blob.object_key
        )));
    }
    Ok(bytes)
}

pub fn delete_source_blobs<'a>(
    store: &dyn ObjectStore,
    blobs: impl IntoIterator<Item = &'a SourceBlob>,
) -> Result<(), ApiError> {
    let mut keys = blobs
        .into_iter()
        .map(|blob| blob.object_key.as_str())
        .collect::<Vec<_>>();
    keys.sort_unstable();
    keys.dedup();
    for key in keys {
        store.delete(key)?;
    }
    Ok(())
}

fn random_object_id() -> Result<String, ApiError> {
    let mut bytes = [0_u8; 32];
    getrandom::fill(&mut bytes).map_err(|error| {
        ApiError::internal_message(format!("object key generation failed: {error}"))
    })?;
    Ok(hex::encode(bytes))
}

fn git_blob_oid(bytes: &[u8]) -> String {
    let mut hasher = Sha1::new();
    hasher.update(format!("blob {}\0", bytes.len()).as_bytes());
    hasher.update(bytes);
    hex::encode(hasher.finalize())
}
