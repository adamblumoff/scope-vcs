//! HTTP wire shapes shared by the cache service and its runners.
//!
//! Authorization/signing implementations live outside this crate. This crate
//! only defines the claims that a signer protects and the cache endpoint DTOs.

mod cache;
mod grant;

pub use cache::*;
pub use grant::*;

pub const RESTORE_CACHE_PATH: &str = "/v1/caches/restore";
pub const PREPARE_CACHE_UPLOAD_PATH: &str = "/v1/caches/uploads/prepare";
pub const COMMIT_CACHE_UPLOAD_PATH: &str = "/v1/caches/uploads/commit";
