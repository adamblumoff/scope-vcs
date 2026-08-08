use super::{
    CacheIdentity, CacheLocation, CachePlatform, PinnedContainerImage,
    location::{CACHE_FORMAT, runner_namespace, volume_name},
    require_real_directory,
};
use anyhow::{Context, bail};
use scope_domain::runs::cache::{CacheNamespace, WorkflowCache};
use serde::{Deserialize, Serialize};
use std::{
    fs::{self, File},
    io::Write,
    path::Path,
};

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub(super) struct CacheRecord {
    pub(super) format: u8,
    pub(super) runner_id: String,
    pub(super) runner_namespace: String,
    pub(super) identity_digest: String,
    pub(super) repository_id: String,
    pub(super) namespace: CacheNamespace,
    pub(super) cache_name: String,
    pub(super) image: String,
    pub(super) container_image: String,
    pub(super) platform: String,
    pub(super) volume_name: String,
    pub(super) state: CacheState,
    pub(super) last_used_at_unix: u64,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "state", rename_all = "kebab-case")]
pub(super) enum CacheState {
    Ready { attempt_id: String },
    Tainted { attempt_id: String },
}

pub(super) fn record_location(root: &Path, record: &CacheRecord) -> CacheLocation {
    CacheLocation::from_namespace(
        root,
        record.runner_namespace.clone(),
        &record.identity_digest,
    )
}

pub(super) fn write_record(root: &Path, record: &CacheRecord) -> anyhow::Result<()> {
    let location = record_location(root, record);
    let metadata = root.join("metadata");
    require_real_directory(&metadata, false, "cache metadata directory")?;
    let directory = location
        .record_path
        .parent()
        .context("cache record path has no runner namespace")?;
    require_real_directory(directory, true, "runner cache metadata namespace")?;
    let temporary = location
        .record_path
        .with_extension(format!("tmp.{}", std::process::id()));
    let bytes = serde_json::to_vec(record)?;
    let mut file = File::create(&temporary)?;
    file.write_all(&bytes)?;
    file.write_all(b"\n")?;
    file.sync_all()?;
    fs::rename(&temporary, &location.record_path)?;
    File::open(directory)?.sync_all()?;
    File::open(metadata)?.sync_all()?;
    Ok(())
}

pub(super) fn read_record_candidate(
    location: &CacheLocation,
    runner_id: &str,
) -> Option<CacheRecord> {
    let record: CacheRecord =
        serde_json::from_slice(&fs::read(&location.record_path).ok()?).ok()?;
    (record.runner_id == runner_id
        && record.runner_namespace == location.runner_namespace
        && valid_record(&record, &location.identity_digest))
    .then_some(record)
}

pub(super) fn metadata_allows_warm(existing: &CacheRecord, desired: &CacheRecord) -> bool {
    matches!(&existing.state, CacheState::Ready { .. })
        && existing.format == desired.format
        && existing.runner_id == desired.runner_id
        && existing.runner_namespace == desired.runner_namespace
        && existing.identity_digest == desired.identity_digest
        && existing.repository_id == desired.repository_id
        && existing.namespace == desired.namespace
        && existing.cache_name == desired.cache_name
        && existing.image == desired.image
        && existing.container_image == desired.container_image
        && existing.platform == desired.platform
        && existing.volume_name == desired.volume_name
}

fn valid_record(record: &CacheRecord, expected_digest: &str) -> bool {
    let identity_matches = || {
        let cache = WorkflowCache::parse(record.cache_name.clone()).ok()?;
        let image = PinnedContainerImage::parse(record.container_image.clone()).ok()?;
        let identity = CacheIdentity::new(
            record.repository_id.clone(),
            record.namespace.clone(),
            cache,
            &image,
            CachePlatform::LinuxAmd64,
        )
        .ok()?;
        (identity.digest() == expected_digest && image.digest() == record.image).then_some(())
    };
    record.format == CACHE_FORMAT
        && !record.runner_id.is_empty()
        && record.runner_namespace == runner_namespace(&record.runner_id)
        && record.identity_digest == expected_digest
        && record.platform == CachePlatform::LinuxAmd64.as_str()
        && record.identity_digest.len() == 64
        && record
            .identity_digest
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit())
        && identity_matches().is_some()
        && record.volume_name == volume_name(&record.runner_namespace, &record.identity_digest)
}

pub(super) fn load_runner_records(
    root: &Path,
    runner_id: &str,
) -> anyhow::Result<Vec<CacheRecord>> {
    let namespace = runner_namespace(runner_id);
    let metadata = root.join("metadata");
    require_real_directory(&metadata, false, "cache metadata directory")?;
    let directory = metadata.join(&namespace);
    if !directory.exists() {
        return Ok(Vec::new());
    }
    require_real_directory(&directory, false, "runner cache metadata namespace")?;
    let mut records = Vec::new();
    for entry in fs::read_dir(directory)? {
        let entry = entry?;
        if entry.file_type()?.is_file()
            && entry.path().extension().and_then(|value| value.to_str()) == Some("json")
        {
            let record: CacheRecord = serde_json::from_slice(&fs::read(entry.path())?)?;
            if record.runner_id != runner_id
                || record.runner_namespace != namespace
                || !valid_record(&record, &record.identity_digest)
                || entry.path().file_stem().and_then(|value| value.to_str())
                    != Some(&record.identity_digest)
            {
                bail!("runner cache metadata identity is invalid");
            }
            records.push(record);
        }
    }
    Ok(records)
}

pub(super) fn find_record_for_volume(
    root: &Path,
    volume: &str,
    runner_id: &str,
) -> anyhow::Result<Option<CacheRecord>> {
    Ok(load_runner_records(root, runner_id)?
        .into_iter()
        .find(|record| record.volume_name == volume))
}
