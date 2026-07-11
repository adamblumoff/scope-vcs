use crate::{
    error::ApiError,
    git::{import::safe_repo_key, storage::git_repo_storage_root},
    persistence::unix_now,
    state::AppState,
};
use sha2::{Digest, Sha256};
use std::{
    fs::{self, OpenOptions},
    io::{ErrorKind, Write},
    path::{Path, PathBuf},
    sync::Mutex,
    thread,
    time::{Duration, Instant},
};

const LOCK_TIMEOUT: Duration = Duration::from_secs(10);
const LOCK_RETRY: Duration = Duration::from_millis(10);
const STALE_LOCK_AFTER_SECS: u64 = 30 * 60;
static STALE_LOCK_REMOVAL: Mutex<()> = Mutex::new(());

pub(super) struct GitLockFile {
    path: PathBuf,
}

impl Drop for GitLockFile {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
}

pub(super) fn acquire_request_ref_update_lock(
    state: &AppState,
    owner: &str,
    repo_name: &str,
    request_ref: &str,
) -> Result<GitLockFile, ApiError> {
    let path = request_ref_update_lock_path(state, owner, repo_name, request_ref);
    acquire_git_lock(path, "request branch update already in progress")
}

pub(super) fn acquire_request_ref_store_lock(
    state: &AppState,
    owner: &str,
    repo_name: &str,
) -> Result<GitLockFile, ApiError> {
    acquire_git_lock(
        request_ref_store_lock_path(state, owner, repo_name),
        "request branch store initialization already in progress",
    )
}

fn acquire_git_lock(
    path: PathBuf,
    conflict_message: &'static str,
) -> Result<GitLockFile, ApiError> {
    acquire_git_lock_with_stale_cleanup(path, conflict_message, true)
}

fn acquire_git_lock_with_stale_cleanup(
    path: PathBuf,
    conflict_message: &'static str,
    stale_cleanup: bool,
) -> Result<GitLockFile, ApiError> {
    if let Some(parent) = path.parent() {
        crate::persistence::ensure_private_dir(parent)?;
    }
    let started_at = Instant::now();
    loop {
        match OpenOptions::new().write(true).create_new(true).open(&path) {
            Ok(mut file) => {
                if let Err(error) = writeln!(
                    file,
                    "pid={}\ncreated_at_unix={}",
                    std::process::id(),
                    unix_now()?
                ) {
                    let _ = fs::remove_file(&path);
                    return Err(ApiError::internal(error));
                }
                return Ok(GitLockFile { path });
            }
            Err(error) if error.kind() == ErrorKind::AlreadyExists => {
                if stale_cleanup && remove_stale_git_lock(&path)? {
                    continue;
                }
                if started_at.elapsed() >= LOCK_TIMEOUT {
                    return Err(ApiError::conflict(conflict_message));
                }
                thread::sleep(LOCK_RETRY);
            }
            Err(error) => return Err(ApiError::internal(error)),
        }
    }
}

fn remove_stale_git_lock(path: &Path) -> Result<bool, ApiError> {
    let _guard = STALE_LOCK_REMOVAL.lock().map_err(|_| {
        ApiError::internal(std::io::Error::other(
            "stale git lock removal mutex poisoned",
        ))
    })?;
    let _recovery_lock = acquire_git_lock_with_stale_cleanup(
        stale_git_lock_recovery_path(path),
        "request branch lock recovery already in progress",
        false,
    )?;
    if !git_lock_is_stale(path)? {
        return Ok(false);
    }
    match fs::remove_file(path) {
        Ok(()) => Ok(true),
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(true),
        Err(error) => Err(ApiError::internal(error)),
    }
}

fn stale_git_lock_recovery_path(path: &Path) -> PathBuf {
    let mut recovery_path = path.to_path_buf();
    let file_name = path
        .file_name()
        .map(|value| value.to_string_lossy())
        .unwrap_or_default();
    recovery_path.set_file_name(format!("{file_name}.recovery"));
    recovery_path
}

pub(super) fn git_lock_is_stale(path: &Path) -> Result<bool, ApiError> {
    let text = match fs::read_to_string(path) {
        Ok(text) => text,
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(true),
        Err(_) => String::new(),
    };
    let pid = lock_field(&text, "pid").and_then(|value| value.parse::<u32>().ok());
    if let Some(pid) = pid
        && !process_is_alive(pid)
    {
        return Ok(true);
    }
    let created_at_unix =
        lock_field(&text, "created_at_unix").and_then(|value| value.parse::<u64>().ok());
    if let Some(created_at_unix) = created_at_unix {
        return Ok(unix_now()?.saturating_sub(created_at_unix) >= STALE_LOCK_AFTER_SECS);
    }
    let modified_at = fs::metadata(path)
        .and_then(|metadata| metadata.modified())
        .map_err(ApiError::internal)?;
    Ok(modified_at
        .elapsed()
        .map(|elapsed| elapsed.as_secs() >= STALE_LOCK_AFTER_SECS)
        .unwrap_or(false))
}

fn lock_field<'a>(text: &'a str, name: &str) -> Option<&'a str> {
    let prefix = format!("{name}=");
    text.lines()
        .find_map(|line| line.strip_prefix(prefix.as_str()))
}

#[cfg(target_os = "linux")]
fn process_is_alive(pid: u32) -> bool {
    PathBuf::from(format!("/proc/{pid}")).exists()
}

#[cfg(all(unix, not(target_os = "linux")))]
fn process_is_alive(pid: u32) -> bool {
    std::process::Command::new("kill")
        .arg("-0")
        .arg(pid.to_string())
        .status()
        .is_ok_and(|status| status.success())
}

#[cfg(not(unix))]
fn process_is_alive(_pid: u32) -> bool {
    true
}

fn request_ref_store_lock_path(state: &AppState, owner: &str, repo_name: &str) -> PathBuf {
    let repo_key = safe_repo_key(owner, repo_name);
    git_repo_storage_root(state)
        .join("git-request-refs-locks")
        .join(format!("{repo_key}-store.lock"))
}

fn request_ref_update_lock_path(
    state: &AppState,
    owner: &str,
    repo_name: &str,
    request_ref: &str,
) -> PathBuf {
    let repo_key = safe_repo_key(owner, repo_name);
    let ref_hash = hex::encode(Sha256::digest(request_ref.as_bytes()));
    git_repo_storage_root(state)
        .join("git-request-refs-locks")
        .join(format!("{repo_key}-{ref_hash}.lock"))
}
