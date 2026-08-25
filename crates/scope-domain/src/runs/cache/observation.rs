use super::definition::validate_cache_name;
use crate::{
    error::DomainError,
    runs::workflow::{definition::WorkflowJobId, identity::WorkflowPath},
};
use serde::{Deserialize, Serialize};

pub const MAX_CACHE_OBSERVATION_DURATION_MS: u64 = 24 * 60 * 60 * 1_000;
pub const MAX_CACHE_OBSERVATION_SIZE_BYTES: u64 = 1024 * 1024 * 1024;

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum CacheColdReason {
    MetadataMissing,
    MetadataInvalid,
    MetadataNotReady,
    VolumeMissing,
    VolumeInvalid,
    BackingDirectoryMissing,
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum CachePreparation {
    Exact,
    Compatible,
    Cold { reason: CacheColdReason },
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum CacheFinalState {
    Pending,
    Ready,
    Evicted,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AttemptCacheSetupObservation {
    pub attempt_id: String,
    pub authorization_ms: u64,
    pub wall_ms: u64,
}

impl AttemptCacheSetupObservation {
    pub fn new(
        attempt_id: impl Into<String>,
        authorization_ms: u64,
        wall_ms: u64,
    ) -> Result<Self, DomainError> {
        let attempt_id = attempt_id.into();
        if attempt_id.trim().is_empty() {
            return Err(DomainError::invalid_input(
                "cache setup observation attempt id is required",
            ));
        }
        validate_observation_duration(authorization_ms)?;
        validate_observation_duration(wall_ms)?;
        if authorization_ms > wall_ms {
            return Err(DomainError::invalid_input(
                "cache authorization duration cannot exceed cache setup wall duration",
            ));
        }
        Ok(Self {
            attempt_id,
            authorization_ms,
            wall_ms,
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AttemptCachePreparationTiming {
    pub key_ms: u64,
    pub metadata_ms: u64,
    pub size_bytes: u64,
    pub download_verify_ms: u64,
    pub sync_ms: u64,
    pub extraction_ms: u64,
    pub prepare_ms: u64,
}

impl AttemptCachePreparationTiming {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        key_ms: u64,
        metadata_ms: u64,
        size_bytes: u64,
        download_verify_ms: u64,
        sync_ms: u64,
        extraction_ms: u64,
        prepare_ms: u64,
    ) -> Result<Self, DomainError> {
        for duration in [
            key_ms,
            metadata_ms,
            download_verify_ms,
            sync_ms,
            extraction_ms,
            prepare_ms,
        ] {
            validate_observation_duration(duration)?;
        }
        if size_bytes > MAX_CACHE_OBSERVATION_SIZE_BYTES {
            return Err(DomainError::invalid_input(
                "cache observation size exceeds the maximum cache object size",
            ));
        }
        let derived_prepare_ms = key_ms
            .checked_add(metadata_ms)
            .and_then(|total| total.checked_add(download_verify_ms))
            .and_then(|total| total.checked_add(sync_ms))
            .and_then(|total| total.checked_add(extraction_ms))
            .ok_or_else(|| DomainError::invalid_input("cache preparation duration overflow"))?;
        if prepare_ms != derived_prepare_ms {
            return Err(DomainError::invalid_input(
                "cache preparation duration must equal the sum of its measured phases",
            ));
        }
        Ok(Self {
            key_ms,
            metadata_ms,
            size_bytes,
            download_verify_ms,
            sync_ms,
            extraction_ms,
            prepare_ms,
        })
    }
}

/// Durable facts observed by a runner for one cache during one attempt.
///
/// The workflow namespace is supplied by the claimed attempt, not by the runner
/// report, so a report cannot move a cache observation across jobs.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AttemptCacheObservation {
    pub attempt_id: String,
    pub workflow_path: WorkflowPath,
    pub job_key: WorkflowJobId,
    pub cache_name: String,
    pub identity_digest: String,
    pub preparation: CachePreparation,
    pub timing: AttemptCachePreparationTiming,
    pub final_state: CacheFinalState,
    pub finalize_ms: Option<u64>,
}

impl AttemptCacheObservation {
    pub fn prepared(
        attempt_id: impl Into<String>,
        workflow_path: WorkflowPath,
        job_key: WorkflowJobId,
        cache_name: impl Into<String>,
        identity_digest: impl Into<String>,
        preparation: CachePreparation,
        timing: AttemptCachePreparationTiming,
    ) -> Result<Self, DomainError> {
        let attempt_id = attempt_id.into();
        if attempt_id.trim().is_empty() {
            return Err(DomainError::invalid_input(
                "cache observation attempt id is required",
            ));
        }
        let cache_name = cache_name.into();
        validate_cache_name(&cache_name).map_err(DomainError::invalid_input)?;
        let identity_digest = identity_digest.into();
        if identity_digest.len() != 64
            || !identity_digest
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        {
            return Err(DomainError::invalid_input(
                "cache observation identity digest must be 64 lowercase hexadecimal characters",
            ));
        }
        AttemptCachePreparationTiming::new(
            timing.key_ms,
            timing.metadata_ms,
            timing.size_bytes,
            timing.download_verify_ms,
            timing.sync_ms,
            timing.extraction_ms,
            timing.prepare_ms,
        )?;
        Ok(Self {
            attempt_id,
            workflow_path,
            job_key,
            cache_name,
            identity_digest,
            preparation,
            timing,
            final_state: CacheFinalState::Pending,
            finalize_ms: None,
        })
    }

    #[allow(clippy::too_many_arguments)]
    pub fn restore(
        attempt_id: impl Into<String>,
        workflow_path: WorkflowPath,
        job_key: WorkflowJobId,
        cache_name: impl Into<String>,
        identity_digest: impl Into<String>,
        preparation: CachePreparation,
        timing: AttemptCachePreparationTiming,
        final_state: CacheFinalState,
        finalize_ms: Option<u64>,
    ) -> Result<Self, DomainError> {
        let mut observation = Self::prepared(
            attempt_id,
            workflow_path,
            job_key,
            cache_name,
            identity_digest,
            preparation,
            timing,
        )?;
        match (final_state, finalize_ms) {
            (CacheFinalState::Pending, None) => {}
            (CacheFinalState::Ready | CacheFinalState::Evicted, Some(duration)) => {
                observation.finalize(final_state, duration)?;
            }
            _ => {
                return Err(DomainError::invalid_input(
                    "cache final state and duration are inconsistent",
                ));
            }
        }
        Ok(observation)
    }

    /// Exact retries are idempotent; a different terminal report is a conflict.
    pub fn finalize(
        &mut self,
        state: CacheFinalState,
        finalize_ms: u64,
    ) -> Result<bool, DomainError> {
        if state == CacheFinalState::Pending {
            return Err(DomainError::invalid_input(
                "cache finalization must be ready or evicted",
            ));
        }
        validate_observation_duration(finalize_ms)?;
        match (self.final_state, self.finalize_ms) {
            (CacheFinalState::Pending, None) => {
                self.final_state = state;
                self.finalize_ms = Some(finalize_ms);
                Ok(true)
            }
            (existing_state, Some(existing_ms))
                if existing_state == state && existing_ms == finalize_ms =>
            {
                Ok(false)
            }
            _ => Err(DomainError::conflict(
                "cache observation already finalized with different facts",
            )),
        }
    }

    pub fn has_same_preparation(&self, other: &Self) -> bool {
        self.attempt_id == other.attempt_id
            && self.workflow_path == other.workflow_path
            && self.job_key == other.job_key
            && self.cache_name == other.cache_name
            && self.identity_digest == other.identity_digest
            && self.preparation == other.preparation
            && self.timing == other.timing
    }
}

fn validate_observation_duration(duration_ms: u64) -> Result<(), DomainError> {
    if duration_ms > MAX_CACHE_OBSERVATION_DURATION_MS {
        return Err(DomainError::invalid_input(
            "cache observation duration exceeds the maximum job duration",
        ));
    }
    Ok(())
}
