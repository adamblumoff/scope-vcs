mod encrypted;
mod memory;
mod s3;
mod source_blobs;

use crate::{config::non_empty_env, error::ApiError};

pub use encrypted::EncryptedObjectStore;
pub use memory::MemoryObjectStore;
pub use s3::S3ObjectStore;
pub use source_blobs::{
    delete_source_blobs, put_repo_object, put_source_blob, repo_object_for_bytes,
    source_blob_bytes, source_blob_text,
};

pub trait ObjectStore: Send + Sync {
    fn put(&self, key: &str, bytes: &[u8]) -> Result<(), ApiError>;
    fn get(&self, key: &str) -> Result<Vec<u8>, ApiError>;
    fn delete(&self, key: &str) -> Result<(), ApiError>;
    fn readiness_check(&self) -> Result<(), ApiError> {
        Ok(())
    }
}

fn required_env(name: &str) -> anyhow::Result<String> {
    non_empty_env(name).ok_or_else(|| anyhow::anyhow!("{name} is required"))
}
