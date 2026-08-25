use anyhow::Context as _;
use scope_domain::runs::cache::definition::CacheKeyInputs;
use sha2::{Digest as _, Sha256};
use std::{fs, io::Read, os::unix::fs::OpenOptionsExt as _, path::Path};

pub(super) const MAX_CACHE_KEY_FILE_BYTES: u64 = 8 * 1024 * 1024;
pub(super) fn digest_inputs(
    inputs: &CacheKeyInputs,
    environment: &std::collections::BTreeMap<String, String>,
    git_oid: &str,
) -> anyhow::Result<String> {
    digest_inputs_at(inputs, environment, Path::new("."), git_oid)
}

pub(super) fn digest_inputs_at(
    inputs: &CacheKeyInputs,
    environment: &std::collections::BTreeMap<String, String>,
    root: &Path,
    git_oid: &str,
) -> anyhow::Result<String> {
    let mut digest = Sha256::new();
    digest.update(b"scope-cache-inputs-v1");
    for path in inputs.files() {
        update_component(&mut digest, "file");
        update_component(&mut digest, path);
        match open_key_file(root, path)? {
            Some((mut file, size)) => {
                update_component(&mut digest, "present");
                digest.update(size.to_be_bytes());
                let mut buffer = [0_u8; 64 * 1024];
                loop {
                    let read = file
                        .read(&mut buffer)
                        .with_context(|| format!("read cache key input {path}"))?;
                    if read == 0 {
                        break;
                    }
                    digest.update(&buffer[..read]);
                }
            }
            None => {
                update_component(&mut digest, "missing");
            }
        }
    }
    for name in inputs.environment() {
        update_component(&mut digest, "environment");
        update_component(&mut digest, name);
        match environment.get(name) {
            Some(value) => {
                update_component(&mut digest, "present");
                update_component(&mut digest, value);
            }
            None => update_component(&mut digest, "missing"),
        }
    }
    if inputs.includes_source() {
        update_component(&mut digest, "source");
        update_component(&mut digest, git_oid);
    }
    Ok(hex::encode(digest.finalize()))
}

pub(super) fn open_key_file(
    root: &Path,
    relative: &str,
) -> anyhow::Result<Option<(fs::File, u64)>> {
    let mut path = root.to_path_buf();
    let mut components = Path::new(relative).components().peekable();
    while let Some(component) = components.next() {
        path.push(component);
        let metadata = match fs::symlink_metadata(&path) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(error) => {
                return Err(error)
                    .with_context(|| format!("inspect cache key input {}", path.display()));
            }
        };
        if components.peek().is_some() {
            if !metadata.file_type().is_dir() {
                anyhow::bail!(
                    "cache key input {} traverses a non-directory or symlink",
                    path.display()
                );
            }
        } else if !metadata.file_type().is_file() {
            anyhow::bail!("cache key input {} is not a regular file", path.display());
        }
    }
    let metadata = fs::symlink_metadata(&path)
        .with_context(|| format!("inspect cache key input {}", path.display()))?;
    if metadata.len() > MAX_CACHE_KEY_FILE_BYTES {
        anyhow::bail!(
            "cache key input {} exceeds {MAX_CACHE_KEY_FILE_BYTES} bytes",
            path.display()
        );
    }
    let file = fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW)
        .open(&path)
        .with_context(|| format!("open cache key input {}", path.display()))?;
    let opened_metadata = file.metadata()?;
    if !opened_metadata.is_file() {
        anyhow::bail!("cache key input {} is not a regular file", path.display());
    }
    if opened_metadata.len() > MAX_CACHE_KEY_FILE_BYTES {
        anyhow::bail!(
            "cache key input {} exceeds {MAX_CACHE_KEY_FILE_BYTES} bytes",
            path.display()
        );
    }
    Ok(Some((file, opened_metadata.len())))
}

fn update_component(digest: &mut Sha256, value: &str) {
    digest.update((value.len() as u64).to_be_bytes());
    digest.update(value.as_bytes());
}
