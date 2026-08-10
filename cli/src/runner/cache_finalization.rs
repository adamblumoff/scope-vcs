use super::{
    CacheRecord, CacheState, RunnerConfig, find_record_for_volume, inspect_volume, lifecycle_lock,
    lock_recorded_volume_identities, record_location, remove_cache, unix_now, volume_is_referenced,
    write_record,
};
use anyhow::bail;
use scope_api_contract::AttemptCacheFinalizationReport;
use scope_domain::runs::cache::CacheFinalState;
use std::{path::Path, process::Command, time::Instant};

pub(super) struct CacheFinalizationTiming {
    pub(super) volume_name: String,
    pub(super) identity_digest: Option<String>,
    pub(super) finalize_ms: u64,
}

pub(in crate::runner) fn finalize_volume_names(
    config: &RunnerConfig,
    volumes: &[String],
    attempt_id: &str,
    success: bool,
) -> anyhow::Result<Vec<AttemptCacheFinalizationReport>> {
    if volumes.is_empty() {
        return Ok(Vec::new());
    }
    let root = super::usable_root(config)?;
    let _identity_locks = lock_recorded_volume_identities(&root, &config.runner_id, volumes)?;
    Ok(
        finalize_volume_names_at_root(config, &root, volumes, attempt_id, success)?
            .into_iter()
            .filter_map(|timing| {
                timing
                    .identity_digest
                    .map(|identity_digest| AttemptCacheFinalizationReport {
                        identity_digest,
                        final_state: if success {
                            CacheFinalState::Ready
                        } else {
                            CacheFinalState::Evicted
                        },
                        finalize_ms: timing.finalize_ms,
                    })
            })
            .collect(),
    )
}

pub(super) fn finalize_volume_names_while_identity_locked(
    config: &RunnerConfig,
    volumes: &[String],
    attempt_id: &str,
    success: bool,
) -> anyhow::Result<Vec<CacheFinalizationTiming>> {
    if volumes.is_empty() {
        return Ok(Vec::new());
    }
    let root = super::usable_root(config)?;
    finalize_volume_names_at_root(config, &root, volumes, attempt_id, success)
}

fn finalize_volume_names_at_root(
    config: &RunnerConfig,
    root: &Path,
    volumes: &[String],
    attempt_id: &str,
    success: bool,
) -> anyhow::Result<Vec<CacheFinalizationTiming>> {
    let _lock = lifecycle_lock(root)?;
    let mut timings = Vec::with_capacity(volumes.len());
    for volume in volumes {
        let started = Instant::now();
        if volume_is_referenced(volume)? {
            bail!("cache volume {volume} is still referenced by a container");
        }
        let record = find_record_for_volume(root, volume, &config.runner_id)?;
        let identity_digest = record.as_ref().map(|record| record.identity_digest.clone());
        let volume_exists = inspect_volume(volume)?.is_some();
        match cache_finalization_action(record.as_ref(), attempt_id, success, volume_exists)? {
            CacheFinalizationAction::Publish => {
                let mut record = record.expect("publish action requires cache metadata");
                let backing = record_location(root, &record).backing_path;
                super::super::command_success(
                    Command::new("sync").args(["-f"]).arg(&backing),
                    "flush successful cache contents",
                )?;
                record.state = CacheState::Ready {
                    attempt_id: attempt_id.to_string(),
                };
                record.last_used_at_unix = unix_now();
                write_record(root, &record)?;
            }
            CacheFinalizationAction::Evict => {
                remove_cache(root, volume, &config.runner_id)?;
            }
            CacheFinalizationAction::Complete => {}
        }
        timings.push(CacheFinalizationTiming {
            volume_name: volume.clone(),
            identity_digest,
            finalize_ms: super::elapsed_millis(started),
        });
    }
    Ok(timings)
}

#[derive(Debug, Eq, PartialEq)]
pub(super) enum CacheFinalizationAction {
    Publish,
    Evict,
    Complete,
}

pub(super) fn cache_finalization_action(
    record: Option<&CacheRecord>,
    attempt_id: &str,
    success: bool,
    volume_exists: bool,
) -> anyhow::Result<CacheFinalizationAction> {
    let Some(record) = record else {
        return if !success && !volume_exists {
            Ok(CacheFinalizationAction::Complete)
        } else {
            bail!("cache finalization metadata is missing")
        };
    };
    let owned = match &record.state {
        CacheState::Ready { attempt_id: owner } | CacheState::Tainted { attempt_id: owner } => {
            owner == attempt_id
        }
    };
    if !owned {
        bail!(
            "cache volume {} is not owned by attempt {attempt_id}",
            record.volume_name
        );
    }
    if !success {
        return Ok(CacheFinalizationAction::Evict);
    }
    if !volume_exists {
        bail!("successful cache volume {} is missing", record.volume_name);
    }
    match record.state {
        CacheState::Ready { .. } => Ok(CacheFinalizationAction::Complete),
        CacheState::Tainted { .. } => Ok(CacheFinalizationAction::Publish),
    }
}
