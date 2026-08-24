#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CacheObjectRecord {
    pub repository_id: String,
    pub checksum_sha256: String,
    pub storage_backend: String,
    pub object_key: String,
    pub size_bytes: u64,
    pub created_at_unix: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CacheRestoreKind {
    Exact,
    Compatible,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CacheRestoreRecord {
    pub source: CacheRestoreKind,
    pub object: CacheObjectRecord,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CacheUploadRecord {
    pub upload_id: String,
    pub repository_id: String,
    pub identity_digest: String,
    pub compatibility_group_digest: String,
    pub checksum_sha256: String,
    pub storage_backend: String,
    pub object_key: String,
    pub size_bytes: u64,
    pub state: CacheUploadState,
    pub created_at_unix: u64,
    pub expires_at_unix: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CacheUploadState {
    Active,
    Deleting,
    Committed,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum CachePrepareResult {
    UseObject {
        object: CacheObjectRecord,
        expires_at_unix: u64,
    },
    Upload(CacheUploadRecord),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum CacheCommitResult {
    Committed {
        object: CacheObjectRecord,
        expires_at_unix: u64,
    },
    AlreadyCommitted {
        object: CacheObjectRecord,
        expires_at_unix: u64,
    },
    Stale {
        orphaned_object_key: String,
    },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PendingCacheDeletion {
    pub repository_id: String,
    pub checksum_sha256: String,
    pub object_key: String,
    pub attempts: u32,
    pub eligible_after_unix: u64,
}
