use super::{CACHE_FORMAT, load_records, remove_cache, volume_is_referenced};
use crate::runner::command_stdout;
use anyhow::{Context, bail};
use serde::{Deserialize, Serialize};
use std::{
    fs::{self, File},
    io::Write,
    path::Path,
    process::Command,
};

const MIN_FREE_BYTES: u64 = 2 * 1024 * 1024 * 1024;
const MIN_FREE_INODES: u64 = 10_000;

#[derive(Debug, Deserialize, Serialize)]
struct StoreIdentity {
    format: u8,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct Capacity {
    pub(super) available_bytes: u64,
    pub(super) available_inodes: Option<u64>,
}

pub(super) fn validate_store(root: &Path, initialize: bool) -> anyhow::Result<Capacity> {
    if initialize {
        fs::create_dir_all(root)
            .with_context(|| format!("create cache root {}", root.display()))?;
    }
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
    let identity = StoreIdentity {
        format: CACHE_FORMAT,
    };
    let identity_path = root.join("store.json");
    if identity_path.exists() {
        let stored: StoreIdentity = serde_json::from_slice(&fs::read(&identity_path)?)?;
        if stored.format != identity.format {
            bail!("runner cache format is unsupported; reinstall the runner to reset its cache");
        }
    } else if initialize {
        let mut entries = fs::read_dir(root)?
            .filter_map(Result::ok)
            .filter(|entry| entry.file_name() != "lost+found")
            .collect::<Vec<_>>();
        entries.retain(|entry| entry.file_name() != ".lifecycle.lock");
        if !entries.is_empty() {
            bail!("new cache directory must be empty");
        }
        let mut file = File::create(&identity_path)?;
        serde_json::to_writer(&mut file, &identity)?;
        file.write_all(b"\n")?;
        file.sync_all()?;
        File::open(root)?.sync_all()?;
    } else {
        bail!("cache directory has not been initialized by runner install");
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
    parse_capacity(&output)
}

fn parse_capacity(output: &str) -> anyhow::Result<Capacity> {
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
        available_inodes: match fields.next().context("cache inode capacity is missing")? {
            "-" | "0" => None,
            available => Some(available.parse()?),
        },
    })
}

pub(super) fn has_capacity(root: &Path) -> anyhow::Result<bool> {
    let capacity = filesystem_capacity(root)?;
    Ok(capacity.available_bytes >= MIN_FREE_BYTES
        && capacity
            .available_inodes
            .is_none_or(|available| available >= MIN_FREE_INODES))
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

#[cfg(test)]
mod tests {
    use super::{Capacity, parse_capacity};

    #[test]
    fn dynamic_inode_counts_are_an_unavailable_metric() {
        assert_eq!(
            parse_capacity("Avail IFree\n123 -\n").unwrap(),
            Capacity {
                available_bytes: 123,
                available_inodes: None,
            }
        );
        assert_eq!(
            parse_capacity("Avail IFree\n123 0\n").unwrap(),
            Capacity {
                available_bytes: 123,
                available_inodes: None,
            }
        );
    }
}
