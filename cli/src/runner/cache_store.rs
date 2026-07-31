use super::{CACHE_FORMAT, load_records, remove_cache, volume_is_referenced};
use crate::runner::{command_stdout, runner_work_root};
use anyhow::{Context, bail};
use serde::{Deserialize, Serialize};
use std::{
    fs::{self, File},
    io::Write,
    os::unix::fs::MetadataExt,
    path::Path,
    process::Command,
};

const MIN_FREE_BYTES: u64 = 2 * 1024 * 1024 * 1024;
const MIN_FREE_INODES: u64 = 10_000;

#[derive(Debug, Deserialize, Serialize)]
struct StoreIdentity {
    format: u8,
    device: u64,
    source: String,
    filesystem: String,
    filesystem_uuid: String,
}

#[derive(Clone, Copy, Debug)]
pub(super) struct Capacity {
    pub(super) available_bytes: u64,
    pub(super) available_inodes: u64,
}

pub(super) fn validate_store(root: &Path, initialize: bool) -> anyhow::Result<Capacity> {
    let metadata = fs::symlink_metadata(root)
        .with_context(|| format!("inspect configured cache root {}", root.display()))?;
    if !metadata.file_type().is_dir() || metadata.file_type().is_symlink() {
        bail!("cache root must be a real directory: {}", root.display());
    }
    if fs::canonicalize(root)? != root {
        bail!(
            "cache root must be an absolute canonical path: {}",
            root.display()
        );
    }
    let parent = root.parent().context("cache root has no parent")?;
    if metadata.dev() == fs::metadata(parent)?.dev() {
        bail!("cache root must be the mount point of a dedicated filesystem");
    }
    for critical in [Path::new("/"), runner_work_root()?.as_path()] {
        if let Ok(critical) = fs::metadata(critical)
            && critical.dev() == metadata.dev()
        {
            bail!("cache storage shares a filesystem with critical runner state");
        }
    }
    let docker_root = command_stdout(
        Command::new("docker").args(["info", "--format={{.DockerRootDir}}"]),
        "inspect Docker data root",
    )?;
    if fs::metadata(docker_root.trim())?.dev() == metadata.dev() {
        bail!("cache storage must not share Docker's transient filesystem");
    }
    let mount = command_stdout(
        Command::new("findmnt")
            .args(["-n", "-o", "SOURCE,FSTYPE,UUID", "-T"])
            .arg(root),
        "inspect cache filesystem",
    )?;
    let mut fields = mount.split_whitespace();
    let source = fields.next().context("cache mount source is missing")?;
    let filesystem = fields.next().context("cache filesystem type is missing")?;
    let filesystem_uuid = fields.next().context("cache filesystem UUID is missing")?;
    if filesystem_uuid == "-" {
        bail!("cache filesystem must expose a stable UUID");
    }
    if !source.starts_with("/dev/")
        || matches!(
            filesystem,
            "tmpfs" | "overlay" | "btrfs" | "zfs" | "nfs" | "nfs4" | "cifs" | "fuse"
        )
        || source.contains("loop")
        || source.contains("zram")
    {
        bail!("cache root must use a dedicated finite local block filesystem");
    }
    let identity = StoreIdentity {
        format: CACHE_FORMAT,
        device: metadata.dev(),
        source: source.to_string(),
        filesystem: filesystem.to_string(),
        filesystem_uuid: filesystem_uuid.to_string(),
    };
    let identity_path = root.join("store.json");
    if identity_path.exists() {
        let stored: StoreIdentity = serde_json::from_slice(&fs::read(&identity_path)?)?;
        if stored.format != identity.format
            || stored.device != identity.device
            || stored.source != identity.source
            || stored.filesystem != identity.filesystem
            || stored.filesystem_uuid != identity.filesystem_uuid
        {
            bail!("configured cache mount identity changed; refusing cached work");
        }
    } else if initialize {
        let mut entries = fs::read_dir(root)?
            .filter_map(Result::ok)
            .filter(|entry| entry.file_name() != "lost+found")
            .collect::<Vec<_>>();
        entries.retain(|entry| entry.file_name() != ".lifecycle.lock");
        if !entries.is_empty() {
            bail!("new cache filesystem must be empty");
        }
        let mut file = File::create(&identity_path)?;
        serde_json::to_writer(&mut file, &identity)?;
        file.write_all(b"\n")?;
        file.sync_all()?;
        File::open(root)?.sync_all()?;
    } else {
        bail!("cache filesystem has not been initialized by runner install");
    }
    filesystem_capacity(root)
}

fn filesystem_capacity(root: &Path) -> anyhow::Result<Capacity> {
    let output = command_stdout(
        Command::new("df")
            .args(["-B1", "--output=avail,iavail"])
            .arg(root),
        "inspect cache filesystem capacity",
    )?;
    let values = output
        .lines()
        .last()
        .context("cache capacity output is empty")?;
    let mut fields = values.split_whitespace();
    Ok(Capacity {
        available_bytes: fields
            .next()
            .context("cache byte capacity is missing")?
            .parse()?,
        available_inodes: fields
            .next()
            .context("cache inode capacity is missing")?
            .parse()?,
    })
}

pub(super) fn has_capacity(root: &Path) -> anyhow::Result<bool> {
    let capacity = filesystem_capacity(root)?;
    Ok(capacity.available_bytes >= MIN_FREE_BYTES && capacity.available_inodes >= MIN_FREE_INODES)
}

pub(super) fn ensure_capacity(
    root: &Path,
    _lifecycle_lock: &File,
    runner_id: &str,
) -> anyhow::Result<()> {
    if has_capacity(root)? {
        return Ok(());
    }
    prune_root(root, runner_id)?;
    if has_capacity(root)? {
        Ok(())
    } else {
        bail!("cache storage remains below its byte or inode reserve after pruning")
    }
}

fn prune_root(root: &Path, runner_id: &str) -> anyhow::Result<()> {
    let mut records = load_records(root)?;
    records.sort_by_key(|record| record.last_used_at_unix);
    for record in records {
        if volume_is_referenced(&record.volume_name)? {
            continue;
        }
        remove_cache(root, &record.volume_name, runner_id)?;
        if has_capacity(root)? {
            break;
        }
    }
    Ok(())
}
