use anyhow::{Context, bail};
use scope_core::domain::repo_config::{
    ConfigVisibility, REPO_CONFIG_KIND, REPO_CONFIG_PATH, REPO_CONFIG_VERSION, RepoConfig,
};
use std::{fs, path::Path, process::Command};

const WORKTREE_CONFIG_PATH: &str = ".scope/repo.json";

pub fn ensure_scope_repo_config_exists(git_root: &Path) -> anyhow::Result<bool> {
    let path = git_root.join(WORKTREE_CONFIG_PATH);
    if path.exists() {
        return Ok(false);
    }

    let parent = path
        .parent()
        .context("Scope config path is missing a parent directory")?;
    fs::create_dir_all(parent).context("create .scope config directory")?;
    fs::write(&path, default_repo_config_json()).context("write .scope/repo.json")?;
    Ok(true)
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
