use super::{
    CacheRecord, CacheState, RunnerConfig, find_record_for_volume, inspect_volume, lifecycle_lock,
    lock_recorded_volume_identities, record_location, remove_cache, unix_now, volume_is_referenced,
    write_record,
};
use anyhow::bail;
use std::{path::Path, process::Command};

pub(in crate::runner) fn finalize_volume_names(
    config: &RunnerConfig,
    volumes: &[String],
    attempt_id: &str,
    success: bool,
) -> anyhow::Result<()> {
    if volumes.is_empty() {
        return Ok(());
    }
    let root = super::usable_root(config)?;
    let _identity_locks = lock_recorded_volume_identities(&root, &config.runner_id, volumes)?;
    finalize_volume_names_at_root(config, &root, volumes, attempt_id, success)
}

pub(super) fn finalize_volume_names_while_identity_locked(
    config: &RunnerConfig,
    volumes: &[String],
    attempt_id: &str,
    success: bool,
) -> anyhow::Result<()> {
    if volumes.is_empty() {
        return Ok(());
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
) -> anyhow::Result<()> {
    let _lock = lifecycle_lock(root)?;
    for volume in volumes {
        if volume_is_referenced(volume)? {
            bail!("cache volume {volume} is still referenced by a container");
        }
        let record = find_record_for_volume(root, volume, &config.runner_id)?;
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
    }
    Ok(())
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
