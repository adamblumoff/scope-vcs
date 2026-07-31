use super::recovery::mark_recovery_cache_finalization_pending;
use super::{ConclusionReportPending, RunnerConfig, RunnerWorkDir, command_stdout, unix_now};
use crate::api::finalize_attempt_cache;
use anyhow::{Context, bail};
use reqwest::blocking::Client;
use scope_api_contract::{
    AttemptCacheFinalizationOutcome, AttemptCacheFinalizationRequest, ClaimRunResponse,
};
use scope_domain::runs::{
    cache::{CacheIdentity, CachePlatform},
    run::PinnedContainerImage,
};
use serde::{Deserialize, Serialize};
use std::{
    fs::{self, File},
    io::Write,
    path::{Path, PathBuf},
    process::Command,
};

const CACHE_FORMAT: u8 = 1;
const CACHE_LABEL: &str = "scope.cache-format=1";

#[path = "cache_store.rs"]
mod store;
use store::{ensure_capacity, has_capacity, validate_store};

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct CacheMount {
    pub(super) volume_name: String,
    pub(super) target: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
struct CacheRecord {
    format: u8,
    identity_digest: String,
    repository_id: String,
    cache_name: String,
    image: String,
    container_image: String,
    platform: String,
    volume_name: String,
    state: CacheState,
    last_used_at_unix: u64,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
struct VolumeInspection {
    name: String,
    driver: String,
    device: Option<String>,
    volume_type: Option<String>,
    options: Option<String>,
    labels: std::collections::BTreeMap<String, String>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "state", rename_all = "kebab-case")]
enum CacheState {
    Ready,
    Tainted { attempt_id: String },
}

pub(super) struct PreparedCaches {
    config: RunnerConfig,
    mounts: Vec<CacheMount>,
    lock: Option<File>,
    finished: bool,
}

impl PreparedCaches {
    pub(super) fn prepare(
        config: &RunnerConfig,
        claim: &ClaimRunResponse,
        pinned_image: &str,
    ) -> anyhow::Result<Self> {
        if claim.job.workflow.caches().is_empty() {
            return Ok(Self {
                config: config.clone(),
                mounts: Vec::new(),
                lock: None,
                finished: false,
            });
        }
        let root = configured_root(config)?;
        validate_store(&root, false)?;
        let lock = lifecycle_lock(&root)?;
        validate_store(&root, false)?;
        ensure_capacity(&root, &lock, &config.runner_id)?;
        let pinned_image = PinnedContainerImage::parse(pinned_image.to_string())?;
        let mut prepared = Self {
            config: config.clone(),
            mounts: Vec::with_capacity(claim.job.workflow.caches().len()),
            lock: Some(lock),
            finished: false,
        };
        for cache in claim.job.workflow.caches() {
            let identity = CacheIdentity::new(
                claim.job.repository_id.clone(),
                cache.clone(),
                &pinned_image,
                CachePlatform::LinuxAmd64,
            )?;
            let digest = identity.digest();
            let volume_name = volume_name(&digest);
            let backing = root.join("data").join(&digest);
            let record = CacheRecord {
                format: CACHE_FORMAT,
                identity_digest: digest,
                repository_id: identity.repository_id().to_string(),
                cache_name: identity.cache().as_str().to_string(),
                image: identity.image_digest().to_string(),
                container_image: pinned_image.as_str().to_string(),
                platform: identity.platform().as_str().to_string(),
                volume_name: volume_name.clone(),
                state: CacheState::Tainted {
                    attempt_id: claim.attempt_id.clone(),
                },
                last_used_at_unix: unix_now(),
            };
            let existing_record = read_record_candidate(&root, &record.identity_digest);
            let existing_volume = inspect_volume(&volume_name)?;
            let warm = existing_record
                .as_ref()
                .is_some_and(|existing| metadata_allows_warm(existing, &record))
                && existing_volume.as_ref().is_some_and(|volume| {
                    volume_matches(volume, &record, &backing, &config.runner_id)
                })
                && backing_is_real_directory(&backing)?;
            if warm {
                if volume_is_referenced(&volume_name)? {
                    bail!("cache volume {volume_name} is still referenced by a container");
                }
            } else {
                cold_recreate(
                    &root,
                    &record,
                    &backing,
                    existing_volume.as_ref(),
                    &config.runner_id,
                )?;
            }
            if let Err(error) = write_record(&root, &record) {
                if !warm {
                    let recreated = inspect_volume(&volume_name)?;
                    discard_cache_identity(
                        &root,
                        &record,
                        &backing,
                        recreated.as_ref(),
                        &config.runner_id,
                    )?;
                }
                return Err(error.context("persist write-ahead cache taint"));
            }
            prepared.mounts.push(CacheMount {
                volume_name: volume_name.clone(),
                target: cache.mount_path().to_string(),
            });
        }
        Ok(prepared)
    }

    pub(super) fn mounts(&self) -> &[CacheMount] {
        &self.mounts
    }

    pub(super) fn volume_names(&self) -> Vec<String> {
        self.mounts
            .iter()
            .map(|mount| mount.volume_name.clone())
            .collect()
    }

    pub(super) fn confirm_container(&mut self, container_name: &str) -> anyhow::Result<()> {
        verify_container_mounts(container_name, &self.mounts)?;
        self.lock.take();
        Ok(())
    }

    pub(super) fn finish(mut self, success: bool) -> anyhow::Result<()> {
        self.lock.take();
        finalize_volume_names(&self.config, &self.volume_names(), success)?;
        self.finished = true;
        Ok(())
    }

    pub(super) fn preserve(mut self) {
        self.finished = true;
    }
}

impl Drop for PreparedCaches {
    fn drop(&mut self) {
        if self.finished || self.mounts.is_empty() {
            return;
        }
        self.lock.take();
        if let Err(error) = finalize_volume_names(&self.config, &self.volume_names(), false) {
            eprintln!("Could not evict tainted attempt caches: {error:#}");
        }
    }
}

pub(super) fn finalize_volume_names(
    config: &RunnerConfig,
    volumes: &[String],
    success: bool,
) -> anyhow::Result<()> {
    if volumes.is_empty() {
        return Ok(());
    }
    let root = configured_root(config)?;
    validate_store(&root, false)?;
    let _lock = lifecycle_lock(&root)?;
    for volume in volumes {
        if volume_is_referenced(volume)? {
            bail!("cache volume {volume} is still referenced by a container");
        }
        if success {
            let mut record = read_record_for_volume(&root, volume)?;
            let backing = root.join("data").join(&record.identity_digest);
            super::command_success(
                Command::new("sync").args(["-f"]).arg(&backing),
                "flush successful cache contents",
            )?;
            record.state = CacheState::Ready;
            record.last_used_at_unix = unix_now();
            write_record(&root, &record)?;
        } else {
            remove_cache(&root, volume, &config.runner_id)?;
        }
    }
    Ok(())
}

pub(super) fn is_reusable_after_execution(claim: &ClaimRunResponse, success: bool) -> bool {
    success
        && !claim
            .canary_phase
            .is_some_and(|phase| phase.evicts_cache_after_success())
}

pub(super) fn acknowledge_finalization(
    client: &Client,
    config: &RunnerConfig,
    claim: &ClaimRunResponse,
    outcome: AttemptCacheFinalizationOutcome,
) -> anyhow::Result<()> {
    finalize_attempt_cache(
        client,
        &config.api_url,
        &claim.attempt_token,
        &claim.attempt_id,
        &AttemptCacheFinalizationRequest { outcome },
    )
}

pub(super) fn finish_canary_ack(
    client: &Client,
    config: &RunnerConfig,
    claim: &ClaimRunResponse,
    work: &mut RunnerWorkDir,
    outcome: AttemptCacheFinalizationOutcome,
) -> anyhow::Result<()> {
    mark_recovery_cache_finalization_pending(&work.path, claim, outcome.clone()).map_err(
        |error| {
            work.preserve();
            eprintln!("Could not persist pending canary cache acknowledgment: {error:#}");
            ConclusionReportPending
        },
    )?;
    acknowledge_finalization(client, config, claim, outcome).map_err(|error| {
        work.preserve();
        eprintln!("Could not acknowledge finalized canary cache: {error:#}");
        ConclusionReportPending
    })?;
    Ok(())
}

pub(super) fn list(config: &RunnerConfig) -> anyhow::Result<()> {
    let Some(root) = config.cache_root.as_deref() else {
        println!("Cache storage is not configured (set SCOPE_RUNNER_CACHE_ROOT during install)");
        return Ok(());
    };
    validate_store(root, false)?;
    let mut records = load_records(root)?;
    records.sort_by(|left, right| {
        left.repository_id
            .cmp(&right.repository_id)
            .then_with(|| left.cache_name.cmp(&right.cache_name))
    });
    if records.is_empty() {
        println!("No runner caches");
        return Ok(());
    }
    for record in records {
        let state = match record.state {
            CacheState::Ready => "ready".to_string(),
            CacheState::Tainted { attempt_id } => format!("tainted:{attempt_id}"),
        };
        println!(
            "{}\t{}\t{}\t{}",
            record.repository_id, record.cache_name, state, record.volume_name
        );
    }
    Ok(())
}

pub(super) fn prune(config: &RunnerConfig, all: bool) -> anyhow::Result<()> {
    let root = configured_root(config)?;
    validate_store(&root, false)?;
    let _lock = lifecycle_lock(&root)?;
    let mut records = load_records(&root)?;
    records.sort_by_key(|record| record.last_used_at_unix);
    let mut removed = 0_u64;
    for record in records {
        if volume_is_referenced(&record.volume_name)? {
            continue;
        }
        if all || !has_capacity(&root)? {
            remove_cache(&root, &record.volume_name, &config.runner_id)?;
            removed += 1;
        }
        if !all && has_capacity(&root)? {
            break;
        }
    }
    println!("Removed {removed} cache volume(s)");
    Ok(())
}

pub(super) fn doctor(config: &RunnerConfig) -> anyhow::Result<()> {
    let root = configured_root(config)?;
    let capacity = validate_store(&root, false)?;
    println!(
        "✓ cache storage {} ({} GiB free, {} inodes free)",
        root.display(),
        capacity.available_bytes / (1024 * 1024 * 1024),
        capacity.available_inodes
    );
    Ok(())
}

pub(super) fn admit(config: &RunnerConfig) -> anyhow::Result<()> {
    let root = configured_root(config)?;
    validate_store(&root, false)?;
    let lock = lifecycle_lock(&root)?;
    validate_store(&root, false)?;
    ensure_capacity(&root, &lock, &config.runner_id)
}

pub(super) fn initialize(root: &Path) -> anyhow::Result<()> {
    validate_store(root, true)?;
    let _lock = lifecycle_lock(root)?;
    fs::create_dir_all(root.join("metadata"))?;
    fs::create_dir_all(root.join("data"))?;
    File::open(root)?.sync_all()?;
    Ok(())
}

pub(super) fn evict_orphaned_tainted(
    config: &RunnerConfig,
    recoverable_attempts: &std::collections::BTreeSet<String>,
) -> anyhow::Result<()> {
    let root = configured_root(config)?;
    validate_store(&root, false)?;
    let _lock = lifecycle_lock(&root)?;
    for record in load_records(&root)? {
        let CacheState::Tainted { attempt_id } = &record.state else {
            continue;
        };
        if recoverable_attempts.contains(attempt_id) || volume_is_referenced(&record.volume_name)? {
            continue;
        }
        remove_cache(&root, &record.volume_name, &config.runner_id)?;
    }
    Ok(())
}

fn configured_root(config: &RunnerConfig) -> anyhow::Result<PathBuf> {
    config.cache_root.clone().context(
        "runner cache storage is not configured; set SCOPE_RUNNER_CACHE_ROOT and reinstall",
    )
}

fn lifecycle_lock(root: &Path) -> anyhow::Result<File> {
    let lock = File::options()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(root.join(".lifecycle.lock"))
        .context("open cache lifecycle lock")?;
    lock.lock().context("lock cache lifecycle")?;
    Ok(lock)
}

fn create_backing_directory(root: &Path, backing: &Path) -> anyhow::Result<()> {
    let data = root.join("data");
    fs::create_dir_all(&data).context("create cache data directory")?;
    if backing.exists() {
        let metadata = fs::symlink_metadata(backing)?;
        if !metadata.file_type().is_dir() || metadata.file_type().is_symlink() {
            bail!(
                "cache backing path is not a real directory: {}",
                backing.display()
            );
        }
        return Ok(());
    }
    fs::create_dir(backing).context("create cache backing directory")?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(backing, fs::Permissions::from_mode(0o777))?;
    }
    File::open(&data)?.sync_all()?;
    Ok(())
}

fn cold_recreate(
    root: &Path,
    record: &CacheRecord,
    backing: &Path,
    existing_volume: Option<&VolumeInspection>,
    runner_id: &str,
) -> anyhow::Result<()> {
    discard_cache_identity(root, record, backing, existing_volume, runner_id)?;
    create_backing_directory(root, backing)?;
    if let Err(error) = create_volume(record, backing, runner_id) {
        let created = inspect_volume(&record.volume_name)?;
        discard_cache_identity(root, record, backing, created.as_ref(), runner_id)?;
        return Err(error);
    }
    let created = inspect_volume(&record.volume_name)?
        .context("Docker did not retain the newly created Scope cache volume")?;
    if !volume_matches(&created, record, backing, runner_id) {
        discard_cache_identity(root, record, backing, Some(&created), runner_id)?;
        bail!("new cache volume does not match its semantic identity or backing path");
    }
    Ok(())
}

fn create_volume(record: &CacheRecord, backing: &Path, runner_id: &str) -> anyhow::Result<()> {
    let backing = backing
        .to_str()
        .context("cache backing path must be UTF-8")?;
    let mut command = Command::new("docker");
    command.args([
        "volume",
        "create",
        "--driver",
        "local",
        "--opt",
        "type=none",
        "--opt",
        "o=bind",
        "--opt",
        &format!("device={backing}"),
        "--label",
        CACHE_LABEL,
        "--label",
        &format!("scope.cache-key={}", record.identity_digest),
        "--label",
        &format!("scope.repository-id={}", record.repository_id),
        "--label",
        &format!("scope.cache-name={}", record.cache_name),
        "--label",
        &format!("scope.image={}", record.image),
        "--label",
        &format!("scope.platform={}", record.platform),
        "--label",
        &format!("scope.runner-id={runner_id}"),
        &record.volume_name,
    ]);
    super::command_success(&mut command, "create Scope cache volume")
}

fn inspect_volume(volume: &str) -> anyhow::Result<Option<VolumeInspection>> {
    let output = Command::new("docker")
        .args(["volume", "inspect", "--format={{json .}}", volume])
        .output()
        .context("inspect Scope cache volume")?;
    if !output.status.success() {
        if String::from_utf8_lossy(&output.stderr)
            .to_ascii_lowercase()
            .contains("no such")
        {
            return Ok(None);
        }
        bail!(
            "inspect Scope cache volume: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    parse_volume_inspection(&output.stdout).map(Some)
}

fn parse_volume_inspection(bytes: &[u8]) -> anyhow::Result<VolumeInspection> {
    let value: serde_json::Value = serde_json::from_slice(bytes)?;
    let labels = value
        .get("Labels")
        .and_then(serde_json::Value::as_object)
        .into_iter()
        .flatten()
        .filter_map(|(key, value)| Some((key.clone(), value.as_str()?.to_string())))
        .collect();
    Ok(VolumeInspection {
        name: value
            .get("Name")
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default()
            .to_string(),
        driver: value
            .get("Driver")
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default()
            .to_string(),
        device: volume_option(&value, "device"),
        volume_type: volume_option(&value, "type"),
        options: volume_option(&value, "o"),
        labels,
    })
}

fn volume_option(value: &serde_json::Value, name: &str) -> Option<String> {
    value
        .get("Options")
        .and_then(|options| options.get(name))
        .and_then(serde_json::Value::as_str)
        .map(str::to_string)
}

fn volume_is_owned(volume: &VolumeInspection, record: &CacheRecord, runner_id: &str) -> bool {
    volume.name == record.volume_name
        && volume.labels.get("scope.cache-format").map(String::as_str) == Some("1")
        && volume.labels.get("scope.cache-key").map(String::as_str)
            == Some(record.identity_digest.as_str())
        && volume.labels.get("scope.runner-id").map(String::as_str) == Some(runner_id)
}

fn volume_matches(
    volume: &VolumeInspection,
    record: &CacheRecord,
    backing: &Path,
    runner_id: &str,
) -> bool {
    volume_is_owned(volume, record, runner_id)
        && volume.driver == "local"
        && volume.device.as_deref() == backing.to_str()
        && volume.volume_type.as_deref() == Some("none")
        && volume.options.as_deref() == Some("bind")
        && volume.labels.get("scope.repository-id").map(String::as_str)
            == Some(record.repository_id.as_str())
        && volume.labels.get("scope.cache-name").map(String::as_str)
            == Some(record.cache_name.as_str())
        && volume.labels.get("scope.image").map(String::as_str) == Some(record.image.as_str())
        && volume.labels.get("scope.platform").map(String::as_str) == Some(record.platform.as_str())
}

fn discard_cache_identity(
    root: &Path,
    record: &CacheRecord,
    backing: &Path,
    volume: Option<&VolumeInspection>,
    runner_id: &str,
) -> anyhow::Result<()> {
    if let Some(volume) = volume {
        if !volume_is_owned(volume, record, runner_id) {
            bail!(
                "cache volume {} is not owned by this runner identity",
                record.volume_name
            );
        }
        if volume_is_referenced(&record.volume_name)? {
            bail!(
                "cache volume {} is still referenced by a container",
                record.volume_name
            );
        }
        super::command_success(
            Command::new("docker").args(["volume", "rm", &record.volume_name]),
            "remove inconsistent Scope cache volume",
        )?;
    }
    remove_backing_if_present(backing, &record.container_image)?;
    let metadata = record_path(root, &record.identity_digest);
    if metadata.exists() {
        fs::remove_file(metadata)?;
    }
    sync_cache_directories(root)
}

fn remove_backing_if_present(backing: &Path, container_image: &str) -> anyhow::Result<()> {
    if !backing.exists() {
        return Ok(());
    }
    let metadata = fs::symlink_metadata(backing)?;
    if !metadata.file_type().is_dir() || metadata.file_type().is_symlink() {
        bail!("refusing unsafe cache backing path {}", backing.display());
    }
    let mut clear = Command::new("docker");
    configure_backing_clear(&mut clear, backing, container_image)?;
    super::command_success(&mut clear, "clear Scope cache backing as container root")?;
    fs::remove_dir(backing)?;
    Ok(())
}

fn configure_backing_clear(
    command: &mut Command,
    backing: &Path,
    container_image: &str,
) -> anyhow::Result<()> {
    let backing = backing
        .to_str()
        .context("cache backing path must be UTF-8")?;
    if backing.contains(':') {
        bail!("cache backing path cannot contain ':'");
    }
    command.args([
        "run",
        "--rm",
        "--network",
        "none",
        "--read-only",
        "--user",
        "0:0",
        "--entrypoint",
        "/bin/sh",
        "--volume",
        &format!("{backing}:/scope-cache"),
        container_image,
        "-c",
        "rm -rf /scope-cache/* /scope-cache/.[!.]* /scope-cache/..?*",
    ]);
    Ok(())
}

fn backing_is_real_directory(backing: &Path) -> anyhow::Result<bool> {
    match fs::symlink_metadata(backing) {
        Ok(metadata) => Ok(metadata.file_type().is_dir() && !metadata.file_type().is_symlink()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(error).context("inspect cache backing directory"),
    }
}

fn verify_container_mounts(container: &str, expected: &[CacheMount]) -> anyhow::Result<()> {
    if expected.is_empty() {
        return Ok(());
    }
    let raw = command_stdout(
        Command::new("docker").args([
            "container",
            "inspect",
            "--format={{json .Mounts}}",
            container,
        ]),
        "verify Scope cache mounts",
    )?;
    let mounts: Vec<serde_json::Value> = serde_json::from_str(raw.trim())?;
    for cache in expected {
        let present = mounts.iter().any(|mount| {
            mount.get("Type").and_then(|value| value.as_str()) == Some("volume")
                && mount.get("Name").and_then(|value| value.as_str()) == Some(&cache.volume_name)
                && mount.get("Destination").and_then(|value| value.as_str()) == Some(&cache.target)
        });
        if !present {
            bail!("container is missing verified cache mount {}", cache.target);
        }
    }
    Ok(())
}

fn volume_name(digest: &str) -> String {
    format!("scope-cache-v1-{}", &digest[..digest.len().min(40)])
}

fn record_path(root: &Path, digest: &str) -> PathBuf {
    root.join("metadata").join(format!("{digest}.json"))
}

fn write_record(root: &Path, record: &CacheRecord) -> anyhow::Result<()> {
    let directory = root.join("metadata");
    fs::create_dir_all(&directory)?;
    let path = record_path(root, &record.identity_digest);
    let temporary = path.with_extension(format!("tmp.{}", std::process::id()));
    let bytes = serde_json::to_vec(record)?;
    let mut file = File::create(&temporary)?;
    file.write_all(&bytes)?;
    file.write_all(b"\n")?;
    file.sync_all()?;
    fs::rename(&temporary, &path)?;
    File::open(directory)?.sync_all()?;
    Ok(())
}

fn read_record_candidate(root: &Path, digest: &str) -> Option<CacheRecord> {
    let record: CacheRecord =
        serde_json::from_slice(&fs::read(record_path(root, digest)).ok()?).ok()?;
    valid_record(&record, digest).then_some(record)
}

fn metadata_allows_warm(existing: &CacheRecord, desired: &CacheRecord) -> bool {
    matches!(&existing.state, CacheState::Ready)
        && existing.format == desired.format
        && existing.identity_digest == desired.identity_digest
        && existing.repository_id == desired.repository_id
        && existing.cache_name == desired.cache_name
        && existing.image == desired.image
        && existing.container_image == desired.container_image
        && existing.platform == desired.platform
        && existing.volume_name == desired.volume_name
}

fn valid_record(record: &CacheRecord, expected_digest: &str) -> bool {
    record.format == CACHE_FORMAT
        && record.identity_digest == expected_digest
        && record.identity_digest.len() == 64
        && record
            .identity_digest
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit())
        && PinnedContainerImage::parse(record.container_image.clone())
            .is_ok_and(|image| image.digest() == record.image)
        && record.volume_name == volume_name(&record.identity_digest)
}

fn load_records(root: &Path) -> anyhow::Result<Vec<CacheRecord>> {
    let directory = root.join("metadata");
    if !directory.exists() {
        return Ok(Vec::new());
    }
    let mut records = Vec::new();
    for entry in fs::read_dir(directory)? {
        let entry = entry?;
        if entry.file_type()?.is_file()
            && entry.path().extension().and_then(|value| value.to_str()) == Some("json")
        {
            let record: CacheRecord = serde_json::from_slice(&fs::read(entry.path())?)?;
            if !valid_record(&record, &record.identity_digest)
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

fn read_record_for_volume(root: &Path, volume: &str) -> anyhow::Result<CacheRecord> {
    load_records(root)?
        .into_iter()
        .find(|record| record.volume_name == volume)
        .with_context(|| format!("cache metadata for {volume} is missing"))
}

fn volume_is_referenced(volume: &str) -> anyhow::Result<bool> {
    let containers = command_stdout(
        Command::new("docker").args([
            "ps",
            "-a",
            "--format={{.ID}}",
            "--filter",
            &format!("volume={volume}"),
        ]),
        "check cache volume references",
    )?;
    Ok(!containers.trim().is_empty())
}

fn remove_cache(root: &Path, volume: &str, runner_id: &str) -> anyhow::Result<()> {
    if volume_is_referenced(volume)? {
        bail!("cache volume {volume} is attached to a container");
    }
    let Some(record) = load_records(root)?
        .into_iter()
        .find(|record| record.volume_name == volume)
    else {
        if inspect_volume(volume)?.is_none() {
            return Ok(());
        }
        bail!("cache metadata for existing volume {volume} is missing");
    };
    if let Some(inspect) = inspect_volume(volume)? {
        if !volume_is_owned(&inspect, &record, runner_id) {
            bail!("cache volume {volume} is not owned by this runner identity");
        }
        super::command_success(
            Command::new("docker").args(["volume", "rm", volume]),
            "remove Scope cache volume",
        )?;
    }
    let backing = root.join("data").join(&record.identity_digest);
    remove_backing_if_present(&backing, &record.container_image)?;
    fs::remove_file(record_path(root, &record.identity_digest))?;
    sync_cache_directories(root)
}

fn sync_cache_directories(root: &Path) -> anyhow::Result<()> {
    File::open(root.join("data"))?.sync_all()?;
    File::open(root.join("metadata"))?.sync_all()?;
    File::open(root)?.sync_all()?;
    Ok(())
}

#[cfg(test)]
#[path = "cache_tests.rs"]
mod tests;
