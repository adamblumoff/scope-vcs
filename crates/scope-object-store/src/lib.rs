mod encrypted;
mod error;
mod filesystem;
mod memory;
mod s3;
mod source_blobs;

pub use encrypted::EncryptedObjectStore;
pub use error::{ObjectStoreError, ObjectStoreErrorKind};
pub use filesystem::{FileObjectStore, FileObjectStoreSettings};
pub use memory::MemoryObjectStore;
pub use s3::{S3ObjectStore, S3ObjectStoreSettings};
pub use source_blobs::{
    ContentObjectKind, content_object_for_bytes, delete_source_blobs, object_key,
    object_key_for_content_ref, put_content_object, put_source_blob, source_blob_bytes,
    source_blob_bytes_bounded,
};

pub trait ObjectStore: Send + Sync {
    fn put(&self, key: &str, bytes: &[u8]) -> Result<(), ObjectStoreError>;

    fn get(&self, key: &str) -> Result<Vec<u8>, ObjectStoreError>;

    fn get_bounded(&self, key: &str, max_bytes: usize) -> Result<Vec<u8>, ObjectStoreError> {
        let bytes = self.get(key)?;
        ensure_object_size("read", key, bytes.len(), max_bytes)?;
        Ok(bytes)
    }

    fn delete(&self, key: &str) -> Result<(), ObjectStoreError>;

    fn readiness_check(&self) -> Result<(), ObjectStoreError> {
        Ok(())
    }
}

pub fn ensure_object_size(
    operation: &str,
    key: &str,
    bytes: usize,
    max_bytes: usize,
) -> Result<(), ObjectStoreError> {
    if bytes > max_bytes {
        return Err(object_too_large(operation, key, bytes, max_bytes));
    }
    Ok(())
}

pub fn object_too_large(
    operation: &str,
    key: &str,
    bytes: usize,
    max_bytes: usize,
) -> ObjectStoreError {
    ObjectStoreError::payload_too_large(format!(
        "object store {operation} for {key} is too large: {bytes} bytes exceeds {max_bytes} bytes"
    ))
}
