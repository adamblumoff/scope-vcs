use super::{lifecycle_lock, read_record_for_volume, require_real_directory, runner_namespace};
use anyhow::{Context, bail};
use std::{
    fs::{self, File},
    ops::Deref,
    os::unix::fs::MetadataExt,
    path::Path,
};

pub(super) struct CacheFileLock(File);

impl CacheFileLock {
    pub(super) fn new(file: File) -> Self {
        Self(file)
    }
}

impl Deref for CacheFileLock {
    type Target = File;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl Drop for CacheFileLock {
    fn drop(&mut self) {
        let _ = self.0.unlock();
    }
}

#[derive(Default)]
pub(super) struct CacheIdentityLocks(Vec<CacheFileLock>);

impl CacheIdentityLocks {
    pub(super) fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    #[cfg(test)]
    pub(super) fn len(&self) -> usize {
        self.0.len()
    }
}

pub(super) fn lock_cache_identities(
    root: &Path,
    runner_id: &str,
    digests: impl IntoIterator<Item = String>,
) -> anyhow::Result<CacheIdentityLocks> {
    let runner_namespace = runner_namespace(runner_id);
    let lock_namespace = {
        let _lifecycle_lock = lifecycle_lock(root)?;
        let locks = root.join("locks");
        require_real_directory(&locks, false, "cache lock directory")?;
        let namespace = locks.join(&runner_namespace);
        require_real_directory(&namespace, true, "runner cache lock namespace")?;
        File::open(&locks)?.sync_all()?;
        namespace
    };
    let digests = canonical_identity_lock_digests(digests)?;
    let mut locks = CacheIdentityLocks(Vec::with_capacity(digests.len()));
    for digest in digests {
        let path = lock_namespace.join(format!("{digest}.lock"));
        if let Ok(metadata) = fs::symlink_metadata(&path)
            && (!metadata.file_type().is_file() || metadata.file_type().is_symlink())
        {
            bail!(
                "cache identity lock must be a regular file: {}",
                path.display()
            );
        }
        let lock = File::options()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(&path)
            .with_context(|| format!("open cache identity lock {}", path.display()))?;
        let path_metadata = fs::symlink_metadata(&path)?;
        let file_metadata = lock.metadata()?;
        if !path_metadata.file_type().is_file()
            || path_metadata.file_type().is_symlink()
            || path_metadata.dev() != file_metadata.dev()
            || path_metadata.ino() != file_metadata.ino()
        {
            bail!(
                "cache identity lock path changed while opening: {}",
                path.display()
            );
        }
        lock.lock()
            .with_context(|| format!("lock cache identity {digest}"))?;
        locks.0.push(CacheFileLock::new(lock));
    }
    Ok(locks)
}

pub(super) fn lock_recorded_volume_identities(
    root: &Path,
    runner_id: &str,
    volumes: &[String],
) -> anyhow::Result<CacheIdentityLocks> {
    let digests = {
        // Read the volume-to-identity mapping consistently, then release lifecycle
        // before blocking on identity locks. Ownership is checked again under both
        // lock layers by the finalizer.
        let _lifecycle_lock = lifecycle_lock(root)?;
        volumes
            .iter()
            .map(|volume| Ok(read_record_for_volume(root, volume, runner_id)?.identity_digest))
            .collect::<anyhow::Result<Vec<_>>>()?
    };
    lock_cache_identities(root, runner_id, digests)
}

pub(super) fn canonical_identity_lock_digests(
    digests: impl IntoIterator<Item = String>,
) -> anyhow::Result<Vec<String>> {
    let mut digests = digests.into_iter().collect::<Vec<_>>();
    if digests
        .iter()
        .any(|digest| digest.len() != 64 || !digest.bytes().all(|byte| byte.is_ascii_hexdigit()))
    {
        bail!("cache identity lock digest is invalid");
    }
    digests.sort_unstable();
    digests.dedup();
    Ok(digests)
}
