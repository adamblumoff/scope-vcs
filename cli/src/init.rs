use crate::{
    api::{
        RepoInitResponse, api_url, create_repo, display_user, http_client, rollback_created_repo,
    },
    git_repo::{
        ensure_git_repo_ready, install_scope_fetch_auth, run_git, warn_if_dirty_working_tree,
    },
    login::session_from_cache_or_browser,
    repo_config::{
        default_scope_repo_config, ensure_scope_repo_config_exists,
        mark_worktree_scope_repo_config_synced, repo_config_path,
    },
};
use anyhow::{Context, bail};
use std::{
    io::{self, Write},
    path::Path,
    process::Command,
};

pub fn run(name: Option<String>) -> anyhow::Result<()> {
    let git_repo = ensure_git_repo_ready("scope init")?;
    let api_url = api_url();
    let repo_name = match name.as_deref() {
        Some(name) => normalize_repo_name(name)?,
        None => prompt_repo_name(&git_repo.root)?,
    };
    warn_if_dirty_working_tree(&git_repo)?;

    let client = http_client()?;
    let session = session_from_cache_or_browser(&client, &api_url)?;
    eprintln!("Signed in as {}", display_user(&session.user));
    let created = create_repo(&client, &api_url, &session.token, repo_name)?;

    let config_created = match configure_remote(&git_repo.root, &created.init)
        .and_then(|_| ensure_scope_repo_config_exists(&git_repo.root))
        .and_then(|config_created| {
            mark_worktree_scope_repo_config_synced(&git_repo.root, &default_scope_repo_config())?;
            Ok(config_created)
        }) {
        Ok(config_created) => config_created,
        Err(error) => {
            rollback_created_repo(&client, &api_url, &session.token, &created.repo);
            return Err(error);
        }
    };

    println!(
        "Created Scope repo: {}/{}",
        created.repo.owner_handle, created.repo.name
    );
    println!("Configured Git remote: {}", created.init.remote_name);
    println!(
        "{} {}",
        if config_created {
            "Created"
        } else {
            "Using existing"
        },
        repo_config_path()
    );
    println!("Run: scope push");
    Ok(())
}

fn configure_remote(git_root: &Path, init: &RepoInitResponse) -> anyhow::Result<()> {
    let _ = Command::new("git")
        .args(["remote", "remove", &init.remote_name])
        .status();
    run_git(&["remote", "add", &init.remote_name, &init.git_remote_url])?;
    install_scope_fetch_auth(git_root, &init.git_remote_url)
}

fn prompt_repo_name(git_root: &Path) -> anyhow::Result<String> {
    let default = default_repo_name(git_root);
    eprint!("Repository name [{default}]: ");
    io::stderr().flush().ok();

    let mut input = String::new();
    io::stdin()
        .read_line(&mut input)
        .context("read repository name")?;
    let name = if input.trim().is_empty() {
        default
    } else {
        input
    };
    normalize_repo_name(&name)
}

fn default_repo_name(git_root: &Path) -> String {
    git_root
        .file_name()
        .map(|name| name.to_string_lossy().to_string())
        .filter(|name| !name.trim().is_empty())
        .unwrap_or_else(|| "repo".to_string())
}

fn normalize_repo_name(name: &str) -> anyhow::Result<String> {
    let name = name.trim().to_ascii_lowercase();
    if name.is_empty() {
        bail!("repository name is required");
    }
    Ok(name)
}
