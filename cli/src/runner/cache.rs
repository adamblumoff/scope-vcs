use super::recovery::mark_recovery_cache_finalization_pending;
use super::{
    ConclusionReportPending, ExecutionOutcome, RunnerConfig, RunnerWorkDir, command_stdout,
    unix_now,
};
use crate::api::finalize_attempt_cache;
use anyhow::{Context, bail};
use reqwest::blocking::Client;
use scope_api_contract::{
    AttemptCacheFinalizationOutcome, AttemptCacheFinalizationRequest, ClaimRunResponse,
};
use scope_domain::runs::{
    cache::{CacheIdentity, CachePlatform},
    cutover::RunnerProtocolCanaryPhase,
    run::PinnedContainerImage,
};
use serde::{Deserialize, Serialize};
use std::{
    fs::{self, File},
    io::Write,
    path::{Path, PathBuf},
    process::Command,
};

#[path = "cache_location.rs"]
mod location;
use location::{CACHE_FORMAT, CacheLocation, runner_namespace, volume_name};

#[path = "cache_store.rs"]
mod store;
use store::{ensure_capacity, has_capacity, validate_store};

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct CacheMount {
    pub(super) volume_name: String,
    pub(super) target: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
struct CacheRecord {
    format: u8,
    runner_id: String,
    runner_namespace: String,
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

fn record_location(root: &Path, record: &CacheRecord) -> CacheLocation {
    CacheLocation::from_namespace(
        root,
        record.runner_namespace.clone(),
        &record.identity_digest,
    )
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
        let root = usable_root(config)?;
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
            let location = CacheLocation::for_runner(&root, &config.runner_id, &digest);
            let record = CacheRecord {
                format: CACHE_FORMAT,
                runner_id: config.runner_id.clone(),
                runner_namespace: location.runner_namespace.clone(),
                identity_digest: digest,
                repository_id: identity.repository_id().to_string(),
                cache_name: identity.cache().as_str().to_string(),
                image: identity.image_digest().to_string(),
                container_image: pinned_image.as_str().to_string(),
                platform: identity.platform().as_str().to_string(),
                volume_name: location.volume_name.clone(),
                state: CacheState::Tainted {
                    attempt_id: claim.attempt_id.clone(),
                },
                last_used_at_unix: unix_now(),
            };
            let existing_record = read_record_candidate(&location, &config.runner_id);
            let existing_volume = inspect_volume(&location.volume_name)?;
            let warm = existing_record
                .as_ref()
                .is_some_and(|existing| metadata_allows_warm(existing, &record))
                && existing_volume.as_ref().is_some_and(|volume| {
                    volume_matches(volume, &record, &location.backing_path, &config.runner_id)
                })
                && backing_is_real_directory(&location.backing_path)?;
            if warm {
                if volume_is_referenced(&location.volume_name)? {
                    bail!(
                        "cache volume {} is still referenced by a container",
                        location.volume_name
                    );
                }
            } else {
                cold_recreate(
                    &root,
                    &record,
                    &location.backing_path,
                    existing_volume.as_ref(),
                    &config.runner_id,
                )?;
            }
            if let Err(error) = write_record(&root, &record) {
                if !warm {
                    let recreated = inspect_volume(&location.volume_name)?;
                    discard_cache_identity(
                        &root,
                        &record,
                        &location.backing_path,
                        recreated.as_ref(),
                        &config.runner_id,
                    )?;
                }
                return Err(error.context("persist write-ahead cache taint"));
            }
            prepared.mounts.push(CacheMount {
                volume_name: location.volume_name,
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
    let root = usable_root(config)?;
    let _lock = lifecycle_lock(&root)?;
    for volume in volumes {
        if volume_is_referenced(volume)? {
            bail!("cache volume {volume} is still referenced by a container");
        }
        if success {
            let mut record = read_record_for_volume(&root, volume, &config.runner_id)?;
            let backing = record_location(&root, &record).backing_path;
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

pub(super) fn is_reusable_after_execution(
    canary_phase: Option<RunnerProtocolCanaryPhase>,
    outcome: ExecutionOutcome,
) -> bool {
    match (canary_phase, outcome) {
        (None, ExecutionOutcome::Succeeded | ExecutionOutcome::Failed) => true,
        (Some(phase), ExecutionOutcome::Succeeded) => !phase.evicts_cache_after_success(),
        _ => false,
    }
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
    let root = usable_root(config)?;
    let mut records = load_runner_records(&root, &config.runner_id)?;
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
    let root = usable_root(config)?;
    let _lock = lifecycle_lock(&root)?;
    let mut records = load_runner_records(&root, &config.runner_id)?;
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
    let root = usable_root(config)?;
    let capacity = validate_store(&root, false)?;
    println!(
        "✓ cache storage {} ({} GiB free, {} inodes free)",
        root.display(),
        capacity.available_bytes / (1024 * 1024 * 1024),
        capacity
            .available_inodes
            .map_or_else(|| "dynamic".to_string(), |available| available.to_string())
    );
    Ok(())
}

pub(super) fn has_emergency_capacity(config: &RunnerConfig) -> anyhow::Result<bool> {
    let root = configured_root(config)?;
    validate_store(&root, false)?;
    has_capacity(&root)
}

pub(super) fn admit(config: &RunnerConfig) -> anyhow::Result<()> {
    let root = usable_root(config)?;
    let lock = lifecycle_lock(&root)?;
    validate_store(&root, false)?;
    ensure_capacity(&root, &lock, &config.runner_id)
}

pub(super) fn initialize(root: &Path) -> anyhow::Result<()> {
    validate_store(root, true)?;
    let _lock = lifecycle_lock(root)?;
    ensure_store_directories(root)
}

pub(super) fn evict_orphaned_tainted(
    config: &RunnerConfig,
    recoverable_attempts: &std::collections::BTreeSet<String>,
) -> anyhow::Result<()> {
    let root = usable_root(config)?;
    let _lock = lifecycle_lock(&root)?;
    for record in load_runner_records(&root, &config.runner_id)? {
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
    super::config::runner_cache_root(config.cache_root.as_deref())
}

fn usable_root(config: &RunnerConfig) -> anyhow::Result<PathBuf> {
    let root = configured_root(config)?;
    ensure_usable_root(
        &root,
        super::config::runner_cache_root_is_disposable_default(config.cache_root.as_deref()),
    )?;
    Ok(root)
}

fn ensure_usable_root(root: &Path, disposable: bool) -> anyhow::Result<()> {
    match validate_store(root, false) {
        Ok(_) => {
            let _lock = lifecycle_lock(root)?;
            ensure_store_directories(root)
        }
        Err(_) if disposable && store_is_absent_or_empty(root)? => {
            initialize(root)?;
            Ok(())
        }
        Err(error) => Err(error),
    }
}

fn ensure_store_directories(root: &Path) -> anyhow::Result<()> {
    // store.json is synchronized before these directories, so a restart must
    // safely finish initialization without accepting symlinks or other file types.
    require_real_directory(&root.join("metadata"), true, "cache metadata directory")?;
    require_real_directory(&root.join("data"), true, "cache data directory")?;
    File::open(root)?.sync_all()?;
    Ok(())
}

fn store_is_absent_or_empty(root: &Path) -> anyhow::Result<bool> {
    match fs::read_dir(root) {
        Ok(entries) => {
            for entry in entries {
                let entry = entry?;
                if entry.file_name() != ".lifecycle.lock" || !entry.file_type()?.is_file() {
                    return Ok(false);
                }
            }
            Ok(true)
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(true),
        Err(error) => Err(error).context("inspect disposable cache root"),
    }
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
    require_real_directory(&data, false, "cache data directory")?;
    let namespace = backing
        .parent()
        .context("cache backing path has no runner namespace")?;
    require_real_directory(namespace, true, "runner cache data namespace")?;
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
    File::open(namespace)?.sync_all()?;
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
        &format!("scope.cache-format={CACHE_FORMAT}"),
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
    record.runner_id == runner_id
        && record.runner_namespace == runner_namespace(runner_id)
        && volume.name == record.volume_name
        && volume
            .labels
            .get("scope.cache-format")
            .and_then(|format| format.parse::<u8>().ok())
            == Some(CACHE_FORMAT)
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
    let location = record_location(root, record);
    let metadata = location.record_path;
    if metadata.exists() {
        fs::remove_file(metadata)?;
    }
    sync_cache_directories(root, &record.runner_namespace)
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

fn write_record(root: &Path, record: &CacheRecord) -> anyhow::Result<()> {
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

fn read_record_candidate(location: &CacheLocation, runner_id: &str) -> Option<CacheRecord> {
    let record: CacheRecord =
        serde_json::from_slice(&fs::read(&location.record_path).ok()?).ok()?;
    (record.runner_id == runner_id
        && record.runner_namespace == location.runner_namespace
        && valid_record(&record, &location.identity_digest))
    .then_some(record)
}

fn metadata_allows_warm(existing: &CacheRecord, desired: &CacheRecord) -> bool {
    matches!(&existing.state, CacheState::Ready)
        && existing.format == desired.format
        && existing.runner_id == desired.runner_id
        && existing.runner_namespace == desired.runner_namespace
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
        && !record.runner_id.is_empty()
        && record.runner_namespace == runner_namespace(&record.runner_id)
        && record.identity_digest == expected_digest
        && record.identity_digest.len() == 64
        && record
            .identity_digest
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit())
        && PinnedContainerImage::parse(record.container_image.clone())
            .is_ok_and(|image| image.digest() == record.image)
        && record.volume_name == volume_name(&record.runner_namespace, &record.identity_digest)
}

fn load_runner_records(root: &Path, runner_id: &str) -> anyhow::Result<Vec<CacheRecord>> {
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

fn require_real_directory(path: &Path, create: bool, label: &str) -> anyhow::Result<()> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_dir() && !metadata.file_type().is_symlink() => {
            Ok(())
        }
        Ok(_) => bail!("{label} must be a real directory: {}", path.display()),
        Err(error) if create && error.kind() == std::io::ErrorKind::NotFound => {
            fs::create_dir(path).with_context(|| format!("create {label} {}", path.display()))
        }
        Err(error) => Err(error).with_context(|| format!("inspect {label} {}", path.display())),
    }
}

fn read_record_for_volume(
    root: &Path,
    volume: &str,
    runner_id: &str,
) -> anyhow::Result<CacheRecord> {
    load_runner_records(root, runner_id)?
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
    let Some(record) = load_runner_records(root, runner_id)?
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
    let location = record_location(root, &record);
    remove_backing_if_present(&location.backing_path, &record.container_image)?;
    fs::remove_file(location.record_path)?;
    sync_cache_directories(root, &record.runner_namespace)
}

fn sync_cache_directories(root: &Path, runner_namespace: &str) -> anyhow::Result<()> {
    sync_real_directory_if_present(
        &root.join("data").join(runner_namespace),
        "runner cache data namespace",
    )?;
    sync_real_directory_if_present(
        &root.join("metadata").join(runner_namespace),
        "runner cache metadata namespace",
    )?;
    File::open(root.join("data"))?.sync_all()?;
    File::open(root.join("metadata"))?.sync_all()?;
    File::open(root)?.sync_all()?;
    Ok(())
}

fn sync_real_directory_if_present(path: &Path, label: &str) -> anyhow::Result<()> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_dir() && !metadata.file_type().is_symlink() => {
            File::open(path)?.sync_all()?;
            Ok(())
        }
        Ok(_) => bail!("{label} must be a real directory: {}", path.display()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error).with_context(|| format!("inspect {label} {}", path.display())),
    }
}

#[cfg(test)]
#[path = "cache_tests.rs"]
mod tests;
