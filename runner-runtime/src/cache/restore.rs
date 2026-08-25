use super::{
    archive::{extract_archive, reset_cache_directory},
    identity::digest_inputs,
    types::{PreparedCache, elapsed_ms},
};
use crate::api::{RuntimeClient, cache_client::CacheDownloadError};
use anyhow::Context as _;
use scope_api_contract::{
    AttemptCacheKeyMaterial, AttemptCachePreparationReport, CacheColdReason as WireCacheColdReason,
    CachePreparation as WireCachePreparation, ReportAttemptCachePreparationsRequest,
    RunJobResponse,
};
use scope_cache_contract::{CacheRestoreSource, RestoreCacheRequest, RestoreCacheResponse};
use scope_cache_domain::CacheDigest;
use scope_domain::runs::cache::{
    identity::{CacheIdentity, CacheNamespace, CachePlatform},
    observation::{CacheColdReason, CachePreparation},
};
use std::{
    fs,
    path::{Path, PathBuf},
    time::Instant,
};

pub(crate) fn prepare_caches(
    client: &RuntimeClient,
    job: &RunJobResponse,
    definition: &scope_domain::runs::workflow::definition::WorkflowJob,
) -> anyhow::Result<Vec<PreparedCache>> {
    let setup_started = Instant::now();
    let workflow_path =
        scope_domain::runs::workflow::identity::WorkflowPath::parse(job.workflow_path.clone())?;
    let job_key =
        scope_domain::runs::workflow::definition::WorkflowJobId::parse(job.job_key.clone())?;
    let namespace = CacheNamespace::workflow(&workflow_path, &job_key);
    let mut identities = Vec::new();
    let mut key_material = Vec::new();
    for cache in definition.caches() {
        let started = Instant::now();
        let compatibility_inputs_digest = digest_inputs(
            cache.compatibility_inputs(),
            definition.environment(),
            &job.git_oid,
        )?;
        let exact_inputs_digest =
            digest_inputs(cache.exact_inputs(), definition.environment(), &job.git_oid)?;
        let identity = CacheIdentity::new(
            &job.repository_id,
            namespace.clone(),
            cache.clone(),
            CachePlatform::LinuxAmd64,
            &compatibility_inputs_digest,
            &exact_inputs_digest,
        )?;
        key_material.push(AttemptCacheKeyMaterial {
            cache_name: cache.as_str().to_string(),
            compatibility_inputs_digest,
            exact_inputs_digest,
        });
        identities.push((identity, elapsed_ms(started)));
    }
    let authorization_started = Instant::now();
    client.authorize_cache_keys(key_material)?;
    let authorization_ms = elapsed_ms(authorization_started);
    let mut prepared = Vec::new();
    let mut reports = Vec::new();
    for (cache, (identity, key_ms)) in definition.caches().iter().zip(identities) {
        let exact_digest = identity.exact_digest();
        let compatibility_group_digest = identity.compatibility_group_digest();
        let path = PathBuf::from(cache.mount_path());
        fs::create_dir_all(&path)
            .with_context(|| format!("create cache path {}", path.display()))?;
        let restore = restore_cache(client, &exact_digest, &compatibility_group_digest, &path)?;
        let phases = CachePreparationPhases {
            key_ms,
            ..restore.phases
        };
        reports.push(AttemptCachePreparationReport {
            cache_name: cache.as_str().to_string(),
            identity_digest: exact_digest.clone(),
            preparation: wire_cache_preparation(restore.preparation),
            key_ms: phases.key_ms,
            metadata_ms: phases.metadata_ms,
            size_bytes: phases.size_bytes,
            download_verify_ms: phases.download_verify_ms,
            sync_ms: phases.sync_ms,
            extraction_ms: phases.extraction_ms,
            prepare_ms: phases.prepare_ms(),
        });
        prepared.push(PreparedCache {
            exact_digest,
            compatibility_group_digest,
            path,
            exact_hit: restore.exact_hit,
        });
    }
    let wall_ms = elapsed_ms(setup_started);
    if let Err(error) = client.report_cache_preparations(&ReportAttemptCachePreparationsRequest {
        authorization_ms,
        wall_ms,
        caches: reports,
    }) {
        eprintln!("runtime cache preparation reporting skipped: {error:#}");
    }
    Ok(prepared)
}
fn restore_cache(
    client: &RuntimeClient,
    exact_digest: &str,
    compatibility_group_digest: &str,
    destination: &Path,
) -> anyhow::Result<CacheRestore> {
    let exact_identity_digest = CacheDigest::parse(exact_digest.to_string())?;
    let compatibility_group_digest = CacheDigest::parse(compatibility_group_digest.to_string())?;
    let metadata_started = Instant::now();
    let session = match client.restore_cache(&RestoreCacheRequest {
        exact_identity_digest,
        compatibility_group_digest,
    }) {
        Ok(session) => session,
        Err(error) => {
            eprintln!("runtime cache restore unavailable for {exact_digest}: {error:#}");
            return Ok(CacheRestore::cold(
                CacheColdReason::MetadataNotReady,
                CachePreparationPhases {
                    metadata_ms: elapsed_ms(metadata_started),
                    ..CachePreparationPhases::default()
                },
            ));
        }
    };
    let mut phases = CachePreparationPhases {
        metadata_ms: elapsed_ms(metadata_started),
        ..CachePreparationPhases::default()
    };
    let (source, url, checksum, size) = match session {
        RestoreCacheResponse::Hit {
            source,
            object_digest,
            size_bytes,
            download_url,
            ..
        } => (
            source,
            download_url,
            object_digest.as_str().to_string(),
            size_bytes,
        ),
        RestoreCacheResponse::Miss => {
            return Ok(CacheRestore::cold(CacheColdReason::MetadataMissing, phases));
        }
    };
    phases.size_bytes = size;
    let temp_dir = match tempfile::tempdir().context("create cache download directory") {
        Ok(temp_dir) => temp_dir,
        Err(error) => {
            eprintln!("runtime cache restore staging failed for {exact_digest}: {error:#}");
            return Ok(CacheRestore::cold(
                CacheColdReason::MetadataNotReady,
                phases,
            ));
        }
    };
    let archive = temp_dir.path().join("cache.tar.zst");
    let download = client.download_cache(&url, &archive, size, &checksum);
    phases.download_verify_ms = download.download_verify_ms;
    phases.sync_ms = download.sync_ms;
    if let Err(error) = download.outcome {
        let reason = match error {
            CacheDownloadError::Transport(error) => {
                eprintln!("runtime cache restore transport failed for {exact_digest}: {error:#}");
                CacheColdReason::MetadataNotReady
            }
            CacheDownloadError::Invalid(error) => {
                eprintln!("runtime cache restore rejected for {exact_digest}: {error:#}");
                CacheColdReason::MetadataInvalid
            }
        };
        return Ok(CacheRestore::cold(reason, phases));
    }
    let extraction_started = Instant::now();
    let extraction = extract_archive(&archive, destination);
    phases.extraction_ms = elapsed_ms(extraction_started);
    if let Err(error) = extraction {
        reset_cache_directory(destination)?;
        eprintln!("runtime cache restore was corrupt for {exact_digest}: {error:#}");
        return Ok(CacheRestore::cold(CacheColdReason::MetadataInvalid, phases));
    }
    Ok(CacheRestore {
        preparation: match source {
            CacheRestoreSource::Exact => CachePreparation::Exact,
            CacheRestoreSource::Compatible => CachePreparation::Compatible,
        },
        exact_hit: source == CacheRestoreSource::Exact,
        phases,
    })
}
struct CacheRestore {
    preparation: CachePreparation,
    exact_hit: bool,
    phases: CachePreparationPhases,
}

impl CacheRestore {
    fn cold(reason: CacheColdReason, phases: CachePreparationPhases) -> Self {
        Self {
            preparation: CachePreparation::Cold { reason },
            exact_hit: false,
            phases,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) struct CachePreparationPhases {
    pub(super) key_ms: u64,
    pub(super) metadata_ms: u64,
    pub(super) size_bytes: u64,
    pub(super) download_verify_ms: u64,
    pub(super) sync_ms: u64,
    pub(super) extraction_ms: u64,
}

impl CachePreparationPhases {
    pub(super) fn prepare_ms(self) -> u64 {
        [
            self.key_ms,
            self.metadata_ms,
            self.download_verify_ms,
            self.sync_ms,
            self.extraction_ms,
        ]
        .into_iter()
        .fold(0, u64::saturating_add)
    }
}

fn wire_cache_preparation(preparation: CachePreparation) -> WireCachePreparation {
    match preparation {
        CachePreparation::Exact => WireCachePreparation::Exact,
        CachePreparation::Compatible => WireCachePreparation::Compatible,
        CachePreparation::Cold { reason } => WireCachePreparation::Cold {
            reason: match reason {
                CacheColdReason::MetadataMissing => WireCacheColdReason::MetadataMissing,
                CacheColdReason::MetadataInvalid => WireCacheColdReason::MetadataInvalid,
                CacheColdReason::MetadataNotReady => WireCacheColdReason::MetadataNotReady,
                CacheColdReason::VolumeMissing => WireCacheColdReason::VolumeMissing,
                CacheColdReason::VolumeInvalid => WireCacheColdReason::VolumeInvalid,
                CacheColdReason::BackingDirectoryMissing => {
                    WireCacheColdReason::BackingDirectoryMissing
                }
            },
        },
    }
}
