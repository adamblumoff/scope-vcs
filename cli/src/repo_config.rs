use anyhow::{Context, bail};
use scope_core::domain::repo_config::{
    ConfigVisibility, REPO_CONFIG_KIND, REPO_CONFIG_PATH, REPO_CONFIG_VERSION, RepoConfig,
};
use std::{
    fs::{self, OpenOptions},
    io::{self, Write},
    path::Path,
    process::Command,
};

const WORKTREE_CONFIG_PATH: &str = ".scope/repo.json";

pub fn ensure_scope_repo_config_exists(git_root: &Path) -> anyhow::Result<bool> {
    let scope_dir = git_root.join(".scope");
    ensure_safe_worktree_config_directory_exists(&scope_dir)?;

    let path = git_root.join(WORKTREE_CONFIG_PATH);
    match fs::symlink_metadata(&path) {
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
    }
}

pub fn load_worktree_scope_repo_config(git_root: &Path) -> anyhow::Result<RepoConfig> {
    let path = git_root.join(WORKTREE_CONFIG_PATH);
    let bytes = fs::read(&path).context("read .scope/repo.json")?;
    RepoConfig::parse_json(&bytes).context("parse .scope/repo.json")
}

pub fn load_committed_scope_repo_config(git_root: &Path) -> anyhow::Result<RepoConfig> {
    load_scope_repo_config_at_commit(git_root, "HEAD")
}

pub fn load_scope_repo_config_at_commit(
    git_root: &Path,
    commit_oid: &str,
) -> anyhow::Result<RepoConfig> {
    let spec = format!("{commit_oid}:.scope/repo.json");
    let output = Command::new("git")
        .current_dir(git_root)
        .args(["show", &spec])
        .output()
        .context("read committed .scope/repo.json")?;
    if !output.status.success() {
        bail!("commit .scope/repo.json before running scope push");
    }

    RepoConfig::parse_json(&output.stdout).context("parse committed .scope/repo.json")
}

pub fn ensure_scope_repo_config_is_committed(git_root: &Path) -> anyhow::Result<()> {
    let output = Command::new("git")
        .current_dir(git_root)
        .args(["status", "--porcelain", "--", WORKTREE_CONFIG_PATH])
        .output()
        .context("inspect .scope/repo.json status")?;
    if !output.status.success() {
        bail!("git status for .scope/repo.json failed");
    }
    if !output.stdout.is_empty() {
        bail!(".scope/repo.json has uncommitted changes; commit it before running scope push");
    }

    let output = Command::new("git")
        .current_dir(git_root)
        .args(["cat-file", "-e", "HEAD:.scope/repo.json"])
        .output()
        .context("inspect committed .scope/repo.json")?;
    if !output.status.success() {
        bail!("commit .scope/repo.json before running scope push");
    }

    Ok(())
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

pub fn write_worktree_scope_repo_config(
    git_root: &Path,
    config: &RepoConfig,
) -> anyhow::Result<()> {
    config.validate().context("validate .scope/repo.json")?;
    let path = git_root.join(WORKTREE_CONFIG_PATH);
    ensure_safe_worktree_config_path(git_root)?;
    let json = canonical_repo_config_json(config)?;
    fs::write(&path, json).context("write .scope/repo.json")
}

pub fn canonical_repo_config_json(config: &RepoConfig) -> anyhow::Result<String> {
    let mut json = serde_json::to_string_pretty(config).context("serialize .scope/repo.json")?;
    json.push('\n');
    Ok(json)
}

fn default_repo_config_json() -> String {
    format!(
        r#"{{
  "kind": "{REPO_CONFIG_KIND}",
  "version": {REPO_CONFIG_VERSION},
  "visibility": {{
    "default": "private",
    "rules": []
  }},
  "history": {{
    "rewrites": []
  }}
}}
"#
    )
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        env,
        path::PathBuf,
        time::{SystemTime, UNIX_EPOCH},
    };

    struct TempDir {
        path: PathBuf,
    }

    impl TempDir {
        fn new(label: &str) -> Self {
            let mut path = env::temp_dir();
            path.push(format!(
                "scope-repo-config-{label}-{}-{}",
                std::process::id(),
                SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap()
                    .as_nanos()
            ));
            fs::create_dir_all(&path).unwrap();
            Self { path }
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.path);
        }
    }

    #[cfg(unix)]
    #[test]
    fn write_rejects_symlinked_worktree_config_file() {
        use std::os::unix::fs::symlink;

        let dir = TempDir::new("file-symlink");
        fs::create_dir_all(dir.path.join(".scope")).unwrap();
        let target = dir.path.join("outside.json");
        fs::write(&target, default_repo_config_json()).unwrap();
        symlink(&target, dir.path.join(WORKTREE_CONFIG_PATH)).unwrap();
        let config = RepoConfig::parse_json(default_repo_config_json().as_bytes()).unwrap();

        let error = write_worktree_scope_repo_config(&dir.path, &config).unwrap_err();

        assert!(
            error
                .to_string()
                .contains(".scope/repo.json cannot be a symlink")
        );
    }

    #[cfg(unix)]
    #[test]
    fn write_rejects_symlinked_scope_directory() {
        use std::os::unix::fs::symlink;

        let dir = TempDir::new("dir-symlink");
        let target_dir = dir.path.join("outside");
        fs::create_dir_all(&target_dir).unwrap();
        symlink(&target_dir, dir.path.join(".scope")).unwrap();
        let config = RepoConfig::parse_json(default_repo_config_json().as_bytes()).unwrap();

        let error = write_worktree_scope_repo_config(&dir.path, &config).unwrap_err();

        assert!(
            error
                .to_string()
                .contains(".scope config directory cannot be a symlink")
        );
    }

    #[cfg(unix)]
    #[test]
    fn create_rejects_symlinked_worktree_config_file() {
        use std::os::unix::fs::symlink;

        let dir = TempDir::new("create-file-symlink");
        fs::create_dir_all(dir.path.join(".scope")).unwrap();
        let target = dir.path.join("outside.json");
        symlink(&target, dir.path.join(WORKTREE_CONFIG_PATH)).unwrap();

        let error = ensure_scope_repo_config_exists(&dir.path).unwrap_err();

        assert!(
            error
                .to_string()
                .contains(".scope/repo.json cannot be a symlink")
        );
        assert!(!target.exists());
    }

    #[cfg(unix)]
    #[test]
    fn create_rejects_symlinked_scope_directory() {
        use std::os::unix::fs::symlink;

        let dir = TempDir::new("create-dir-symlink");
        let target_dir = dir.path.join("outside");
        fs::create_dir_all(&target_dir).unwrap();
        symlink(&target_dir, dir.path.join(".scope")).unwrap();

        let error = ensure_scope_repo_config_exists(&dir.path).unwrap_err();

        assert!(
            error
                .to_string()
                .contains(".scope config directory cannot be a symlink")
        );
        assert!(!target_dir.join("repo.json").exists());
    }
}
