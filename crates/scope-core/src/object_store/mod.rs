mod encrypted;
mod filesystem;
mod memory;
mod s3;
mod source_blobs;

use crate::{config::non_empty_env, error::ApiError};

pub use encrypted::EncryptedObjectStore;
pub use filesystem::FileObjectStore;
pub use memory::MemoryObjectStore;
pub use s3::S3ObjectStore;
pub use source_blobs::{
    delete_source_blobs, put_repo_object, put_source_blob, repo_object_for_bytes, source_blob_bytes,
};

pub trait ObjectStore: Send + Sync {
    fn put(&self, key: &str, bytes: &[u8]) -> Result<(), ApiError>;
    fn get(&self, key: &str) -> Result<Vec<u8>, ApiError>;
    fn get_bounded(&self, key: &str, max_bytes: usize) -> Result<Vec<u8>, ApiError> {
        let bytes = self.get(key)?;
        ensure_object_size("read", key, bytes.len(), max_bytes)?;
        Ok(bytes)
    }
    fn delete(&self, key: &str) -> Result<(), ApiError>;
    fn readiness_check(&self) -> Result<(), ApiError> {
        Ok(())
    }
}

pub fn ensure_object_size(
    operation: &str,
    key: &str,
    bytes: usize,
    max_bytes: usize,
) -> Result<(), ApiError> {
    if bytes > max_bytes {
        return Err(object_too_large(operation, key, bytes, max_bytes));
    }
    Ok(())
}

pub fn object_too_large(operation: &str, key: &str, bytes: usize, max_bytes: usize) -> ApiError {
    ApiError::payload_too_large(format!(
        "object store {operation} for {key} is too large: {bytes} bytes exceeds {max_bytes} bytes"
    ))
}

fn required_env(name: &str) -> anyhow::Result<String> {
    non_empty_env(name).ok_or_else(|| anyhow::anyhow!("{name} is required"))
}
