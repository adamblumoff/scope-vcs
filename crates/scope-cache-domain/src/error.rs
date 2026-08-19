use thiserror::Error;

#[derive(Clone, Debug, Error, PartialEq, Eq)]
pub enum CacheDomainError {
    #[error("repository id is required")]
    MissingRepositoryId,
    #[error("cache digest must be 64 lowercase hexadecimal characters")]
    InvalidDigest,
    #[error("upload lease id is required")]
    MissingUploadLeaseId,
    #[error("cache object size must be greater than zero")]
    EmptyObject,
    #[error("cache object is {actual_bytes} bytes; the maximum is {maximum_bytes} bytes")]
    ObjectTooLarge {
        actual_bytes: u64,
        maximum_bytes: u64,
    },
    #[error(
        "cache object would use {requested_bytes} bytes for the repository; the maximum is {maximum_bytes} bytes"
    )]
    RepositoryBudgetExceeded {
        requested_bytes: u64,
        maximum_bytes: u64,
    },
    #[error("cache timestamp overflowed")]
    TimestampOverflow,
    #[error("cache byte accounting overflowed")]
    ByteCountOverflow,
    #[error("cache reference belongs to a different repository or logical identity")]
    ReferenceScopeMismatch,
    #[error("cache reference version must be greater than zero")]
    InvalidReferenceVersion,
    #[error("cache reference version overflowed")]
    ReferenceVersionOverflow,
    #[error("cache reference expiry does not match policy")]
    InvalidReferenceExpiry,
    #[error("cache reference access cannot precede its last update")]
    ReferenceAccessBeforeLastUpdate,
    #[error("upload lease expiry does not match policy")]
    InvalidUploadLeaseExpiry,
    #[error("uploaded object does not match its upload lease")]
    UploadLeaseMismatch,
    #[error("upload lease expired")]
    UploadLeaseExpired,
    #[error("upload lease was prepared against a stale cache reference")]
    StaleUploadLease,
}
