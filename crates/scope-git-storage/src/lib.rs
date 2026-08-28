mod envelope;
mod error;
mod file;
mod ingest;
mod lifecycle;
mod memory;
mod multipart;
mod restore;

pub use envelope::{ENCODING_VERSION, SegmentEncryptionKey};
pub use error::{GitStorageError, MultipartError};
pub use file::FileMultipartStore;
pub use memory::MemoryMultipartStore;
pub use multipart::{
    MultipartStore, MultipartUpload, RemoteReader, S3MultipartSettings, S3MultipartStore,
    UploadedPart,
};

use scope_domain::repository::git::GitSegmentRef;
use sha2::{Digest, Sha256};
use std::{
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};
use tokio::fs::File;

const DEFAULT_CHUNK_BYTES: usize = 1024 * 1024;
const DEFAULT_PART_BYTES: usize = 8 * 1024 * 1024;
const DEFAULT_CHANNEL_CAPACITY: usize = 2;

#[derive(Clone, Debug)]
pub struct GitSegmentStoreConfig {
    pub local_root: PathBuf,
    pub chunk_bytes: usize,
    pub multipart_part_bytes: usize,
    pub channel_capacity: usize,
}

impl GitSegmentStoreConfig {
    pub fn new(local_root: impl Into<PathBuf>) -> Self {
        Self {
            local_root: local_root.into(),
            chunk_bytes: DEFAULT_CHUNK_BYTES,
            multipart_part_bytes: DEFAULT_PART_BYTES,
            channel_capacity: DEFAULT_CHANNEL_CAPACITY,
        }
    }

    fn validate(&self, minimum_part_bytes: usize) -> Result<(), GitStorageError> {
        if self.local_root.as_os_str().is_empty() {
            return Err(GitStorageError::InvalidConfiguration(
                "local Git segment root is required".into(),
            ));
        }
        if self.chunk_bytes == 0 || self.chunk_bytes > 16 * 1024 * 1024 {
            return Err(GitStorageError::InvalidConfiguration(
                "Git segment chunk size must be between 1 byte and 16 MiB".into(),
            ));
        }
        if self.multipart_part_bytes < minimum_part_bytes {
            return Err(GitStorageError::InvalidConfiguration(format!(
                "multipart part size must be at least {minimum_part_bytes} bytes"
            )));
        }
        if self.channel_capacity == 0 {
            return Err(GitStorageError::InvalidConfiguration(
                "Git segment channel capacity must be greater than zero".into(),
            ));
        }
        Ok(())
    }
}

#[derive(Clone, Debug)]
pub struct GitSegmentIngestTimings {
    pub total: Duration,
    pub local_write_and_fsync: Duration,
    pub remote_multipart_upload: Duration,
    pub fanout_blocked: Duration,
    pub plaintext_bytes: u64,
    pub encrypted_bytes: u64,
    pub uploaded_parts: u32,
    pub chunk_bytes: usize,
    pub channel_capacity: usize,
}

#[derive(Clone, Debug)]
pub struct GitSegmentRestoreTimings {
    pub total: Duration,
    pub plaintext_bytes: u64,
    pub verified_frames: u32,
    pub source: GitSegmentRestoreSource,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GitSegmentRestoreSource {
    Local,
    Remote,
}

#[derive(Clone, Debug)]
pub struct StagedGitSegment {
    pub segment: GitSegmentRef,
    pub object_key: String,
    pub encrypted_bytes: u64,
    pub key_id: String,
    local_pack_path: PathBuf,
    pub timings: GitSegmentIngestTimings,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GitSegmentReservation {
    pub segment_id: String,
    pub object_key: String,
}

impl StagedGitSegment {
    pub fn local_pack_path(&self) -> &Path {
        &self.local_pack_path
    }

    pub async fn open_local_pack(&self) -> Result<File, GitStorageError> {
        File::open(&self.local_pack_path)
            .await
            .map_err(GitStorageError::Local)
    }
}

pub struct GitSegmentStore {
    backend: Arc<dyn MultipartStore>,
    encryption_key: SegmentEncryptionKey,
    config: GitSegmentStoreConfig,
}

impl Clone for GitSegmentStore {
    fn clone(&self) -> Self {
        Self {
            backend: Arc::clone(&self.backend),
            encryption_key: self.encryption_key.clone(),
            config: self.config.clone(),
        }
    }
}

impl GitSegmentStore {
    pub fn in_memory(
        encryption_key: SegmentEncryptionKey,
        config: GitSegmentStoreConfig,
    ) -> Result<Self, GitStorageError> {
        Self::new(
            Arc::new(MemoryMultipartStore::default()),
            encryption_key,
            config,
        )
    }

    pub fn new(
        backend: Arc<dyn MultipartStore>,
        encryption_key: SegmentEncryptionKey,
        config: GitSegmentStoreConfig,
    ) -> Result<Self, GitStorageError> {
        config.validate(backend.minimum_part_bytes())?;
        Ok(Self {
            backend,
            encryption_key,
            config,
        })
    }

    fn local_directory(&self, repository_id: &str) -> PathBuf {
        let repository_hash = hex::encode(Sha256::digest(repository_id.as_bytes()));
        self.config.local_root.join(&repository_hash[..32])
    }

    fn local_pack_path(&self, repository_id: &str, segment_id: &str) -> PathBuf {
        self.local_directory(repository_id)
            .join(format!("{segment_id}.pack"))
    }
}

pub fn object_key(repository_id: &str, segment_id: &str) -> String {
    let repository_hash = hex::encode(Sha256::digest(repository_id.as_bytes()));
    format!("git/segments/v2/{}/{segment_id}", &repository_hash[..32])
}

fn valid_segment_id(segment_id: &str) -> bool {
    segment_id.len() == 32
        && segment_id
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
}

#[cfg(test)]
mod tests;
