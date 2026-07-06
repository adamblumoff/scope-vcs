use anyhow::{Context, bail};
use scope_core::domain::repo_config::{
    ConfigVisibility, REPO_CONFIG_PATH, RepoConfig, is_repo_config_fingerprint,
    repo_config_fingerprint,
};
use serde::{Deserialize, Serialize};
use std::{
    fs::{self, OpenOptions},
    io::{self, Write},
    path::{Path, PathBuf},
    process::Command,
    time::{SystemTime, UNIX_EPOCH},
};

const WORKTREE_CONFIG_PATH: &str = ".scope/repo.json";
const WORKTREE_CONFIG_STATE_PATH: &str = ".scope/repo-state.json";
const WORKTREE_CONFIG_STATE_KIND: &str = "scope.repo-config-state";
const WORKTREE_CONFIG_STATE_VERSION: u8 = 1;
const LOCAL_ONLY_SCOPE_PATHS: [&str; 2] = [WORKTREE_CONFIG_PATH, WORKTREE_CONFIG_STATE_PATH];

#[derive(Deserialize, Serialize)]
struct WorktreeRepoConfigState {
    kind: String,
    version: u8,
    base_config_hash: String,
}

pub fn ensure_scope_repo_config_exists(git_root: &Path) -> anyhow::Result<bool> {
    let scope_dir = git_root.join(".scope");
    ensure_safe_worktree_config_directory_exists(&scope_dir)?;

    let path = git_root.join(WORKTREE_CONFIG_PATH);
    let created = match fs::symlink_metadata(&path) {
        Ok(metadata) => {
            if metadata.file_type().is_symlink() {
                bail!(".scope/repo.json cannot be a symlink");
            }
            if !metadata.is_file() {
                bail!(".scope/repo.json must be a regular file");
            }
            Ok(false)
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            let mut file = OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&path)
                .context("create .scope/repo.json")?;
            file.write_all(default_repo_config_json().as_bytes())
                .context("write .scope/repo.json")?;
            Ok(true)
        }
        Err(error) => Err(error).context("inspect .scope/repo.json"),
    }?;
    ensure_scope_repo_config_is_locally_excluded(git_root)?;
    Ok(created)
}

pub fn load_worktree_scope_repo_config(git_root: &Path) -> anyhow::Result<RepoConfig> {
    let path = git_root.join(WORKTREE_CONFIG_PATH);
    let bytes = fs::read(&path).context("read .scope/repo.json")?;
    RepoConfig::parse_json(&bytes).context("parse .scope/repo.json")
}

pub fn load_worktree_scope_repo_config_base_hash(git_root: &Path) -> anyhow::Result<String> {
    let path = git_root.join(WORKTREE_CONFIG_STATE_PATH);
    let bytes = fs::read(&path).with_context(|| {
        format!("read {WORKTREE_CONFIG_STATE_PATH}; run scope clone or scope init")
    })?;
    let state: WorktreeRepoConfigState =
        serde_json::from_slice(&bytes).context("parse .scope/repo-state.json")?;
    if state.kind != WORKTREE_CONFIG_STATE_KIND {
        bail!(".scope/repo-state.json kind must be {WORKTREE_CONFIG_STATE_KIND}");
    }
    if state.version != WORKTREE_CONFIG_STATE_VERSION {
        bail!(".scope/repo-state.json version must be {WORKTREE_CONFIG_STATE_VERSION}");
    }
    if !is_repo_config_fingerprint(&state.base_config_hash) {
        bail!(".scope/repo-state.json base_config_hash must be a SHA-256 hex digest");
    }
    Ok(state.base_config_hash)
}

pub fn config_visibility_label(visibility: ConfigVisibility) -> &'static str {
    match visibility {
        ConfigVisibility::Private => "private",
        ConfigVisibility::Public => "public",
    }
}

pub fn repo_config_path() -> &'static str {
    REPO_CONFIG_PATH.trim_start_matches('/')
}

pub fn default_scope_repo_config() -> RepoConfig {
    RepoConfig::with_default_visibility(ConfigVisibility::Private)
}

pub fn write_worktree_scope_repo_config(
    git_root: &Path,
    config: &RepoConfig,
) -> anyhow::Result<()> {
    config.validate().context("validate .scope/repo.json")?;
    let path = git_root.join(WORKTREE_CONFIG_PATH);
    ensure_safe_worktree_config_path(git_root)?;
    let json = canonical_repo_config_json(config)?;
    write_config_atomically(&path, &json)?;
    ensure_scope_repo_config_is_locally_excluded(git_root)
}

pub fn write_worktree_scope_repo_config_with_base(
    git_root: &Path,
    config: &RepoConfig,
) -> anyhow::Result<()> {
    write_worktree_scope_repo_config(git_root, config)?;
    mark_worktree_scope_repo_config_synced(git_root, config)
}

pub fn mark_worktree_scope_repo_config_synced(
    git_root: &Path,
    config: &RepoConfig,
) -> anyhow::Result<()> {
    config.validate().context("validate .scope/repo.json")?;
    ensure_safe_worktree_config_state_path(git_root)?;
    let base_config_hash =
        repo_config_fingerprint(config).context("fingerprint .scope/repo.json")?;
    let state = WorktreeRepoConfigState {
        kind: WORKTREE_CONFIG_STATE_KIND.to_string(),
        version: WORKTREE_CONFIG_STATE_VERSION,
        base_config_hash,
    };
    let mut json =
        serde_json::to_string_pretty(&state).context("serialize .scope/repo-state.json")?;
    json.push('\n');
    write_config_atomically(&git_root.join(WORKTREE_CONFIG_STATE_PATH), &json)?;
    ensure_scope_repo_config_is_locally_excluded(git_root)
}

pub fn canonical_repo_config_json(config: &RepoConfig) -> anyhow::Result<String> {
    let mut json = serde_json::to_string_pretty(config).context("serialize .scope/repo.json")?;
    json.push('\n');
    Ok(json)
}

fn default_repo_config_json() -> String {
    canonical_repo_config_json(&default_scope_repo_config()).expect("default repo config is valid")
}

fn ensure_safe_worktree_config_path(git_root: &Path) -> anyhow::Result<()> {
    let scope_dir = git_root.join(".scope");
    ensure_safe_worktree_config_directory_exists(&scope_dir)?;

    let config_path = git_root.join(WORKTREE_CONFIG_PATH);
    match fs::symlink_metadata(&config_path) {
        Ok(metadata) => {
            if metadata.file_type().is_symlink() {
                bail!(".scope/repo.json cannot be a symlink");
            }
            if !metadata.is_file() {
                bail!(".scope/repo.json must be a regular file");
            }
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(error) => return Err(error).context("inspect .scope/repo.json"),
    }

    Ok(())
}

fn ensure_safe_worktree_config_state_path(git_root: &Path) -> anyhow::Result<()> {
    let scope_dir = git_root.join(".scope");
    ensure_safe_worktree_config_directory_exists(&scope_dir)?;

    let state_path = git_root.join(WORKTREE_CONFIG_STATE_PATH);
    match fs::symlink_metadata(&state_path) {
        Ok(metadata) => {
            if metadata.file_type().is_symlink() {
                bail!(".scope/repo-state.json cannot be a symlink");
            }
            if !metadata.is_file() {
                bail!(".scope/repo-state.json must be a regular file");
            }
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(error) => return Err(error).context("inspect .scope/repo-state.json"),
    }

    Ok(())
}

fn ensure_safe_worktree_config_directory_exists(scope_dir: &Path) -> anyhow::Result<()> {
    match fs::symlink_metadata(scope_dir) {
        Ok(metadata) => {
            if metadata.file_type().is_symlink() {
                bail!(".scope config directory cannot be a symlink");
            }
            if !metadata.is_dir() {
                bail!(".scope config path must be a directory");
            }
            Ok(())
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            fs::create_dir(scope_dir).context("create .scope config directory")?;
            Ok(())
        }
        Err(error) => Err(error).context("inspect .scope config directory"),
    }
}

fn write_config_atomically(path: &Path, contents: &str) -> anyhow::Result<()> {
    let parent = path
        .parent()
        .context("Scope config path is missing a parent directory")?;
    let temp_path = temporary_config_path(parent)?;
    let result = (|| -> anyhow::Result<()> {
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temp_path)
            .context("create temporary .scope/repo.json")?;
        file.write_all(contents.as_bytes())
            .context("write temporary .scope/repo.json")?;
        file.sync_all().context("sync temporary .scope/repo.json")?;
        drop(file);
        fs::rename(&temp_path, path).context("replace .scope/repo.json")?;
        Ok(())
    })();

    if result.is_err() {
        let _ = fs::remove_file(&temp_path);
    }

    result
}

fn temporary_config_path(parent: &Path) -> anyhow::Result<PathBuf> {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .context("system clock is before Unix epoch")?
        .as_nanos();
    Ok(parent.join(format!(".repo.json.{}.{}.tmp", std::process::id(), nanos)))
}

fn ensure_scope_repo_config_is_locally_excluded(git_root: &Path) -> anyhow::Result<()> {
    let Some(exclude_path) = git_info_exclude_path(git_root)? else {
        return Ok(());
    };
    if let Some(parent) = exclude_path.parent() {
        fs::create_dir_all(parent).context("create Git info directory")?;
    }
    let existing = fs::read_to_string(&exclude_path).unwrap_or_default();
    let missing = LOCAL_ONLY_SCOPE_PATHS
        .iter()
        .copied()
        .filter(|path| !existing.lines().map(str::trim).any(|line| line == *path))
        .collect::<Vec<_>>();
    if missing.is_empty() {
        return Ok(());
    }

    let mut file = OpenOptions::new()
        .append(true)
        .create(true)
        .open(&exclude_path)
        .with_context(|| format!("open {}", exclude_path.display()))?;
    if !existing.is_empty() && !existing.ends_with('\n') {
        file.write_all(b"\n")
            .with_context(|| format!("update {}", exclude_path.display()))?;
    }
    for path in missing {
        file.write_all(format!("{path}\n").as_bytes())
            .with_context(|| format!("update {}", exclude_path.display()))?;
    }
    Ok(())
}

fn git_info_exclude_path(git_root: &Path) -> anyhow::Result<Option<PathBuf>> {
    let output = Command::new("git")
        .current_dir(git_root)
        .args(["rev-parse", "--git-path", "info/exclude"])
        .output()
        .context("resolve Git exclude path")?;
    if !output.status.success() {
        let fallback = git_root.join(".git/info/exclude");
        if fallback.parent().is_some_and(Path::is_dir) {
            return Ok(Some(fallback));
        }
        return Ok(None);
    }

    let path = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if path.is_empty() {
        bail!("Git exclude path could not be determined");
    }
    let path = PathBuf::from(path);
    if path.is_absolute() {
        Ok(Some(path))
    } else {
        Ok(Some(git_root.join(path)))
    }
}

#[cfg(test)]
#[path = "repo_config_tests.rs"]
mod tests;
