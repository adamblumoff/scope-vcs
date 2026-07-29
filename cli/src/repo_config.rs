use anyhow::{Context, bail};
use scope_domain::repo_config::{
    ConfigVisibility, RepoConfig, is_repo_config_fingerprint, repo_config_fingerprint,
};
use serde::{Deserialize, Serialize};
use std::{
    fs::{self, OpenOptions},
    io::{self, Write},
    path::{Path, PathBuf},
    process::Command,
    time::{SystemTime, UNIX_EPOCH},
};

const REPO_CONFIG_FILE: &str = "repo.json";
const REPO_CONFIG_STATE_FILE: &str = "repo-state.json";
const REPO_CONFIG_STATE_KIND: &str = "scope.repo-config-state";
const REPO_CONFIG_STATE_VERSION: u8 = 1;

#[derive(Deserialize, Serialize)]
struct WorktreeRepoConfigState {
    kind: String,
    version: u8,
    base_config_hash: String,
}

struct RepoStatePaths {
    directory: PathBuf,
    config: PathBuf,
    state: PathBuf,
}

pub fn ensure_scope_repo_config_exists(git_root: &Path) -> anyhow::Result<bool> {
    let paths = repo_state_paths(git_root)?;
    ensure_safe_state_directory_exists(&paths.directory)?;
    match fs::symlink_metadata(&paths.config) {
        Ok(metadata) => {
            ensure_regular_file(&metadata, "Scope repo config")?;
            Ok(false)
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            let mut file = OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&paths.config)
                .context("create Scope repo config")?;
            file.write_all(default_repo_config_json().as_bytes())
                .context("write Scope repo config")?;
            Ok(true)
        }
        Err(error) => Err(error).context("inspect Scope repo config"),
    }
}

pub fn load_worktree_scope_repo_config(git_root: &Path) -> anyhow::Result<RepoConfig> {
    let path = repo_config_path(git_root)?;
    let bytes = fs::read(&path).context("read Scope repo config")?;
    RepoConfig::parse_json(&bytes).context("parse Scope repo config")
}

pub fn load_worktree_scope_repo_config_base_hash(git_root: &Path) -> anyhow::Result<String> {
    let path = repo_state_paths(git_root)?.state;
    let bytes =
        fs::read(&path).context("read Scope repo config state; run scope clone or scope init")?;
    let state: WorktreeRepoConfigState =
        serde_json::from_slice(&bytes).context("parse Scope repo config state")?;
    if state.kind != REPO_CONFIG_STATE_KIND {
        bail!("Scope repo config state kind must be {REPO_CONFIG_STATE_KIND}");
    }
    if state.version != REPO_CONFIG_STATE_VERSION {
        bail!("Scope repo config state version must be {REPO_CONFIG_STATE_VERSION}");
    }
    if !is_repo_config_fingerprint(&state.base_config_hash) {
        bail!("Scope repo config state base_config_hash must be a SHA-256 hex digest");
    }
    Ok(state.base_config_hash)
}

pub fn config_visibility_label(visibility: ConfigVisibility) -> &'static str {
    match visibility {
        ConfigVisibility::Private => "private",
        ConfigVisibility::Public => "public",
    }
}

pub fn repo_config_path(git_root: &Path) -> anyhow::Result<PathBuf> {
    Ok(repo_state_paths(git_root)?.config)
}

pub fn default_scope_repo_config() -> RepoConfig {
    RepoConfig::with_default_visibility(ConfigVisibility::Private)
}

pub fn write_worktree_scope_repo_config(
    git_root: &Path,
    config: &RepoConfig,
) -> anyhow::Result<()> {
    config.validate().context("validate Scope repo config")?;
    let paths = repo_state_paths(git_root)?;
    ensure_safe_state_file_path(&paths.directory, &paths.config, "Scope repo config")?;
    let json = canonical_repo_config_json(config)?;
    write_config_atomically(&paths.config, &json, "Scope repo config")
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
    config.validate().context("validate Scope repo config")?;
    let paths = repo_state_paths(git_root)?;
    ensure_safe_state_file_path(&paths.directory, &paths.state, "Scope repo config state")?;
    let base_config_hash =
        repo_config_fingerprint(config).context("fingerprint Scope repo config")?;
    let state = WorktreeRepoConfigState {
        kind: REPO_CONFIG_STATE_KIND.to_string(),
        version: REPO_CONFIG_STATE_VERSION,
        base_config_hash,
    };
    let mut json =
        serde_json::to_string_pretty(&state).context("serialize Scope repo config state")?;
    json.push('\n');
    write_config_atomically(&paths.state, &json, "Scope repo config state")
}

pub fn canonical_repo_config_json(config: &RepoConfig) -> anyhow::Result<String> {
    let mut json = serde_json::to_string_pretty(config).context("serialize Scope repo config")?;
    json.push('\n');
    Ok(json)
}

fn default_repo_config_json() -> String {
    canonical_repo_config_json(&default_scope_repo_config()).expect("default repo config is valid")
}

fn repo_state_paths(git_root: &Path) -> anyhow::Result<RepoStatePaths> {
    let output = Command::new("git")
        .current_dir(git_root)
        .args(["rev-parse", "--git-path", "scope"])
        .output()
        .context("resolve per-worktree Scope state path")?;
    if !output.status.success() {
        bail!(
            "resolve per-worktree Scope state path: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    let value = String::from_utf8(output.stdout)
        .context("per-worktree Scope state path is not UTF-8")?
        .trim()
        .to_string();
    if value.is_empty() {
        bail!("per-worktree Scope state path could not be determined");
    }
    let directory = PathBuf::from(value);
    let directory = if directory.is_absolute() {
        directory
    } else {
        git_root.join(directory)
    };
    Ok(RepoStatePaths {
        config: directory.join(REPO_CONFIG_FILE),
        state: directory.join(REPO_CONFIG_STATE_FILE),
        directory,
    })
}

fn ensure_safe_state_file_path(directory: &Path, path: &Path, label: &str) -> anyhow::Result<()> {
    ensure_safe_state_directory_exists(directory)?;
    match fs::symlink_metadata(path) {
        Ok(metadata) => ensure_regular_file(&metadata, label),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error).with_context(|| format!("inspect {label}")),
    }
}

fn ensure_regular_file(metadata: &fs::Metadata, label: &str) -> anyhow::Result<()> {
    if metadata.file_type().is_symlink() {
        bail!("{label} cannot be a symlink");
    }
    if !metadata.is_file() {
        bail!("{label} must be a regular file");
    }
    Ok(())
}

fn ensure_safe_state_directory_exists(directory: &Path) -> anyhow::Result<()> {
    match fs::symlink_metadata(directory) {
        Ok(metadata) => {
            if metadata.file_type().is_symlink() {
                bail!("Scope repo state directory cannot be a symlink");
            }
            if !metadata.is_dir() {
                bail!("Scope repo state path must be a directory");
            }
            Ok(())
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            fs::create_dir(directory).context("create Scope repo state directory")
        }
        Err(error) => Err(error).context("inspect Scope repo state directory"),
    }
}

fn write_config_atomically(path: &Path, contents: &str, label: &str) -> anyhow::Result<()> {
    let parent = path
        .parent()
        .context("Scope repo state path is missing a parent directory")?;
    let temp_path = temporary_config_path(parent, path)?;
    let result = (|| -> anyhow::Result<()> {
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temp_path)
            .with_context(|| format!("create temporary {label}"))?;
        file.write_all(contents.as_bytes())
            .with_context(|| format!("write temporary {label}"))?;
        file.sync_all()
            .with_context(|| format!("sync temporary {label}"))?;
        drop(file);
        fs::rename(&temp_path, path).with_context(|| format!("replace {label}"))
    })();

    if result.is_err() {
        let _ = fs::remove_file(&temp_path);
    }
    result
}

fn temporary_config_path(parent: &Path, path: &Path) -> anyhow::Result<PathBuf> {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .context("system clock is before Unix epoch")?
        .as_nanos();
    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .context("Scope repo state file has no name")?;
    Ok(parent.join(format!(".{file_name}.{}.{}.tmp", std::process::id(), nanos)))
}

#[cfg(test)]
#[path = "repo_config_tests.rs"]
mod tests;
