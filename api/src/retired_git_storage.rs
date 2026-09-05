//! One-way local storage cutover. Retired names are recognized only here.
use anyhow::{Context, bail, ensure};
use std::{
    fs::{self, File, OpenOptions},
    io::Write,
    path::{Component, Path, PathBuf},
};

const MARKER: &str = ".retired-git-storage-v1.complete";
const COMPLETE: &[u8] = b"retired-git-storage-v1\n";
const LOCK: &str = ".git-storage-writers.lock";

pub async fn scrub_retired_git_storage_for_maintenance(
    database_url: String,
) -> anyhow::Result<usize> {
    let fence = scope_postgres::db::ExclusiveWriterFence::acquire(&database_url).await?;
    let root = crate::config::data_dir(&crate::config::git_repo_root());
    let count = scrub(&root, |_| Ok(()))?;
    fence.release().await?;
    Ok(count)
}

pub(crate) fn open_writer(root: &Path) -> anyhow::Result<File> {
    validate_root(root)?;
    let lock = open_lock(root)?;
    lock.try_lock_shared()
        .context("local Git storage maintenance is running")?;
    if !complete(root)? {
        lock.unlock()?;
        lock.try_lock()
            .context("local Git writers must stop before storage cutover")?;
        return initialize_clean_writer(root, lock);
    }
    Ok(lock)
}

fn initialize_clean_writer(root: &Path, lock: File) -> anyhow::Result<File> {
    ensure!(
        targets(root)?.is_empty(),
        "retired Git storage remains; stop writers and run scope-maintenance scrub-retired-git-storage"
    );
    write_marker(root)?;
    // Forked children can retain this open file description until exec. Closing
    // our descriptor alone would leave its exclusive lock held in those children.
    lock.unlock()?;
    lock.try_lock_shared()
        .context("local Git storage maintenance is running")?;
    Ok(lock)
}

fn validate_root(root: &Path) -> anyhow::Result<()> {
    ensure!(
        root.is_absolute() && root.parent().is_some(),
        "private data root must be an absolute non-root path"
    );
    ensure!(
        root.components()
            .all(|c| matches!(c, Component::RootDir | Component::Normal(_))),
        "private data root must be normalized"
    );
    ensure!(
        fs::canonicalize(root)? == root,
        "private data root must not traverse symlinks"
    );
    let meta = fs::symlink_metadata(root)?;
    ensure!(meta.is_dir(), "private data root must be a directory");
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        ensure!(
            meta.mode() & 0o777 == 0o700 && meta.uid() == unsafe { libc::geteuid() },
            "private data root must be owned by this user with mode 0700"
        );
    }
    Ok(())
}

fn open_lock(root: &Path) -> anyhow::Result<File> {
    let mut options = OpenOptions::new();
    options.read(true).write(true).create(true).truncate(false);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW).mode(0o600);
    }
    let file = options.open(root.join(LOCK))?;
    ensure!(
        file.metadata()?.is_file(),
        "invalid Git storage writer lock"
    );
    Ok(file)
}

fn complete(root: &Path) -> anyhow::Result<bool> {
    let path = root.join(MARKER);
    match fs::symlink_metadata(&path) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(e) => Err(e.into()),
        Ok(meta) => {
            ensure!(
                meta.is_file() && !meta.file_type().is_symlink(),
                "invalid storage cutover marker"
            );
            ensure!(
                fs::read(path)? == COMPLETE,
                "invalid storage cutover marker; investigate before reopening writers"
            );
            Ok(true)
        }
    }
}

fn write_marker(root: &Path) -> anyhow::Result<()> {
    // create_new never follows an injected symlink. A crash before rename leaves
    // this temporary file, which can safely be replaced by the next scrub.
    let temp = root.join(format!("{MARKER}.tmp"));
    if let Ok(meta) = fs::symlink_metadata(&temp) {
        ensure!(
            meta.is_file() && !meta.file_type().is_symlink(),
            "invalid temporary cutover marker"
        );
        fs::remove_file(&temp)?;
    }
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temp)?;
    file.write_all(COMPLETE)?;
    file.sync_all()?;
    fs::rename(temp, root.join(MARKER))?;
    File::open(root)?.sync_all()?;
    Ok(())
}

fn scrub(
    root: &Path,
    mut after_delete: impl FnMut(usize) -> anyhow::Result<()>,
) -> anyhow::Result<usize> {
    validate_root(root)?;
    let lock = open_lock(root)?;
    lock.try_lock()
        .context("local Git writers must stop before storage cutover")?;
    complete(root)?;
    let targets = targets(root)?;
    // Validate the entire deletion plan before removing anything, including
    // nested symlinks and mount points in retired repositories.
    for path in &targets {
        validate_tree(root, path)?;
    }
    for (index, path) in targets.iter().enumerate() {
        if path.is_dir() {
            fs::remove_dir_all(path)?;
        } else {
            fs::remove_file(path)?;
        }
        File::open(path.parent().unwrap())?.sync_all()?;
        tracing::info!(path = %path.display(), "deleted retired local Git storage");
        after_delete(index + 1)?;
    }
    write_marker(root)?;
    Ok(targets.len())
}

fn validate_tree(root: &Path, path: &Path) -> anyhow::Result<()> {
    let meta = fs::symlink_metadata(path)?;
    ensure!(
        !meta.file_type().is_symlink() && (meta.is_dir() || meta.is_file()),
        "unsafe retired storage target: {}",
        path.display()
    );
    ensure!(
        fs::canonicalize(path)?.starts_with(root),
        "out-of-root storage target"
    );
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        ensure!(
            meta.dev() == fs::metadata(root)?.dev(),
            "storage target crosses filesystem boundary"
        );
    }
    if meta.is_dir() {
        for entry in fs::read_dir(path)? {
            validate_tree(root, &entry?.path())?;
        }
    }
    Ok(())
}

fn hex_key(value: &str, length: usize) -> bool {
    value.len() == length
        && value
            .bytes()
            .all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c))
}
fn retired_key(value: &str) -> bool {
    value.strip_prefix("repo-").is_some_and(|v| hex_key(v, 64))
}

fn targets(root: &Path) -> anyhow::Result<Vec<PathBuf>> {
    let mut targets = Vec::new();
    for group in [
        "git-request-refs",
        "git-request-refs-locks",
        "git-rx",
        "git-repos",
        "git-staged",
    ] {
        let directory = root.join(group);
        match fs::symlink_metadata(&directory) {
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
            Err(e) => return Err(e.into()),
            Ok(meta) => ensure!(
                meta.is_dir() && !meta.file_type().is_symlink(),
                "invalid storage directory: {}",
                directory.display()
            ),
        }
        for entry in fs::read_dir(&directory)? {
            let entry = entry?;
            let name = entry.file_name();
            let name = name.to_str().context("non-UTF8 storage target")?;
            let (retired, active) = match group {
                "git-request-refs-locks" => {
                    let stem = name
                        .strip_suffix(".lock")
                        .context("unrecognized Git lock")?;
                    let (key, suffix) = stem.rsplit_once('-').context("unrecognized Git lock")?;
                    let valid = suffix == "store" || hex_key(suffix, 64);
                    (valid && retired_key(key), valid && hex_key(key, 32))
                }
                "git-rx" => {
                    let stem = name
                        .strip_suffix(".git")
                        .context("unrecognized receive staging path")?;
                    let (key, nonce) = stem
                        .split_once('-')
                        .context("unrecognized receive staging path")?;
                    (
                        hex_key(key, 16) && hex_key(nonce, 32),
                        hex_key(key, 32) && hex_key(nonce, 32),
                    )
                }
                _ => {
                    let key = name
                        .strip_suffix(".git")
                        .context("unrecognized Git repository path")?;
                    (
                        retired_key(key),
                        group == "git-request-refs" && hex_key(key, 32),
                    )
                }
            };
            if !retired && !active {
                bail!("ambiguous storage target: {}", entry.path().display());
            }
            let meta = fs::symlink_metadata(entry.path())?;
            ensure!(
                !meta.file_type().is_symlink()
                    && if group == "git-request-refs-locks" {
                        meta.is_file()
                    } else {
                        meta.is_dir()
                    },
                "invalid storage target: {}",
                entry.path().display()
            );
            if retired && !matches!(group, "git-repos" | "git-staged") {
                targets.push(entry.path());
            }
        }
        if matches!(group, "git-repos" | "git-staged") {
            targets.push(directory);
        }
    }
    targets.sort();
    Ok(targets)
}

#[cfg(test)]
mod tests;
