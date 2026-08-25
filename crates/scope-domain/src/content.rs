use crate::content_ref::ContentRef;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SourceBlob {
    pub content_ref: ContentRef,
    pub sha256: String,
    pub git_oid: String,
    pub git_file_mode: String,
    pub size_bytes: u64,
}

pub const DEFAULT_GIT_FILE_MODE: &str = "100644";
pub const EXECUTABLE_GIT_FILE_MODE: &str = "100755";

pub fn is_supported_git_file_mode(mode: &str) -> bool {
    matches!(mode, DEFAULT_GIT_FILE_MODE | EXECUTABLE_GIT_FILE_MODE)
}
