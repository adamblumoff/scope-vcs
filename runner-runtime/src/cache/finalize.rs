use super::{
    archive::{create_archive, file_identity},
    types::{
        CacheFinalization, CacheFinalizationOutcome, CacheSkipReason, PreparedCache, elapsed_ms,
    },
};
use crate::api::RuntimeClient;
use anyhow::Context as _;
use scope_api_contract::{
    AttemptCacheFinalizationReport, CacheFinalState as WireCacheFinalState,
    ReportAttemptCacheFinalizationsRequest,
};
use scope_cache_contract::{
    CommitCacheUploadRequest, PrepareCacheUploadRequest, PrepareCacheUploadResponse,
};
use scope_cache_domain::CacheDigest;
use std::time::Instant;

pub(crate) fn save_caches(
    client: &RuntimeClient,
    caches: &[PreparedCache],
) -> Vec<CacheFinalization> {
    let mut finalizations = Vec::with_capacity(caches.len());
    let mut reports = Vec::new();
    for cache in caches {
        let started = Instant::now();
        let outcome = save_cache(client, cache);
        if matches!(
            outcome,
            CacheFinalizationOutcome::Ready | CacheFinalizationOutcome::Unchanged
        ) {
            reports.push(AttemptCacheFinalizationReport {
                identity_digest: cache.exact_digest.clone(),
                final_state: WireCacheFinalState::Ready,
                finalize_ms: elapsed_ms(started),
            });
        }
        finalizations.push(CacheFinalization {
            identity_digest: cache.exact_digest.clone(),
            outcome,
        });
    }
    if !reports.is_empty()
        && let Err(error) = client
            .report_cache_finalizations(&ReportAttemptCacheFinalizationsRequest { caches: reports })
    {
        eprintln!("runtime cache finalization reporting skipped: {error:#}");
    }
    finalizations
}
pub(super) fn save_cache(
    client: &RuntimeClient,
    cache: &PreparedCache,
) -> CacheFinalizationOutcome {
    if cache.exact_hit {
        return CacheFinalizationOutcome::Unchanged;
    }
    let temp = match tempfile::NamedTempFile::new().context("create cache upload file") {
        Ok(temp) => temp,
        Err(error) => return skipped(CacheSkipReason::ArchiveFailed, error),
    };
    if let Err(error) = create_archive(&cache.path, temp.path()) {
        return skipped(CacheSkipReason::ArchiveFailed, error);
    }
    let (size_bytes, checksum_sha256) = match file_identity(temp.path()) {
        Ok(identity) => identity,
        Err(error) => return skipped(CacheSkipReason::ArchiveFailed, error),
    };
    let exact_identity_digest = match CacheDigest::parse(cache.exact_digest.clone()) {
        Ok(digest) => digest,
        Err(error) => return skipped(CacheSkipReason::ServiceUnavailable, error.into()),
    };
    let object_digest = match CacheDigest::parse(checksum_sha256.clone()) {
        Ok(digest) => digest,
        Err(error) => return skipped(CacheSkipReason::ArchiveFailed, error.into()),
    };
    let session = match client.prepare_cache_upload(&PrepareCacheUploadRequest {
        exact_identity_digest: exact_identity_digest.clone(),
        compatibility_group_digest: match CacheDigest::parse(
            cache.compatibility_group_digest.clone(),
        ) {
            Ok(digest) => digest,
            Err(error) => return skipped(CacheSkipReason::ServiceUnavailable, error.into()),
        },
        object_digest: object_digest.clone(),
        size_bytes,
    }) {
        Ok(session) => session,
        Err(error) => return skipped(CacheSkipReason::ServiceUnavailable, error),
    };
    match session {
        PrepareCacheUploadResponse::UseObject { .. } => CacheFinalizationOutcome::Ready,
        PrepareCacheUploadResponse::Upload {
            lease_id,
            upload_url,
            upload_headers,
            ..
        } => {
            if let Err(error) = client.upload_cache(&upload_url, &upload_headers, temp.path()) {
                return skipped(CacheSkipReason::UploadFailed, error);
            }
            if let Err(error) = client.commit_cache_upload(&CommitCacheUploadRequest {
                lease_id,
                object_digest,
                size_bytes,
            }) {
                return skipped(CacheSkipReason::CommitFailed, error);
            }
            CacheFinalizationOutcome::Ready
        }
    }
}

fn skipped(reason: CacheSkipReason, error: anyhow::Error) -> CacheFinalizationOutcome {
    CacheFinalizationOutcome::Skipped {
        reason,
        message: format!("{error:#}"),
    }
}
