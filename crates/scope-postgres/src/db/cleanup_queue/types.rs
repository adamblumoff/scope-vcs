use scope_domain::{content::SourceBlob, repo_actions::RepoStorageCleanup};
use std::collections::BTreeSet;

#[derive(Clone)]
pub(super) struct LoadedRepoStorageCleanup {
    pub(super) cleanup: RepoStorageCleanup,
    pub(super) generation: String,
}

#[derive(Clone)]
pub(super) struct LoadedSourceBlobCleanup {
    pub(super) blob: SourceBlob,
    pub(super) generation: String,
}

pub struct RepoStorageCleanupBatch {
    pub pending: Vec<RepoStorageCleanup>,
    pub live_repo_ids: BTreeSet<String>,
    pub(super) loaded: Vec<LoadedRepoStorageCleanup>,
}

pub struct SourceBlobCleanupBatch {
    pub pending: Vec<SourceBlob>,
    pub(super) loaded: Vec<LoadedSourceBlobCleanup>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SourceBlobCleanupDecision {
    Delete,
    Referenced,
    StaleClaim,
}

pub struct RepoStorageCleanupClaim {
    pub(super) generation: String,
    pub(super) claim_until: i64,
    pub(crate) cleanup: RepoStorageCleanup,
}
