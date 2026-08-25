use std::{path::PathBuf, time::Instant};

const _: () = assert!(
    scope_domain::runs::cache::observation::MAX_CACHE_OBSERVATION_SIZE_BYTES
        == scope_cache_domain::MAX_CACHE_OBJECT_BYTES
);

pub(crate) struct PreparedCache {
    pub(super) exact_digest: String,
    pub(super) compatibility_group_digest: String,
    pub(super) path: PathBuf,
    pub(super) exact_hit: bool,
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum CacheFinalizationOutcome {
    Ready,
    Unchanged,
    Skipped {
        reason: CacheSkipReason,
        message: String,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum CacheSkipReason {
    ArchiveFailed,
    ServiceUnavailable,
    UploadFailed,
    CommitFailed,
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) struct CacheFinalization {
    pub(crate) identity_digest: String,
    pub(crate) outcome: CacheFinalizationOutcome,
}
pub(crate) fn elapsed_ms(started: Instant) -> u64 {
    u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX)
}
