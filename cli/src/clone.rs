use crate::{
    api::{api_url, get_repo_config, http_client},
    auth::read_stored_session_token,
    git_repo::{clone_with_bearer, install_scope_fetch_auth},
    repo_config::write_worktree_scope_repo_config_with_base,
};
use anyhow::{Context, bail};
use std::path::{Path, PathBuf};

#[derive(Debug, PartialEq, Eq)]
pub struct RepoSpec {
    pub owner: String,
    pub repo: String,
}

pub fn clone_repo(repository: &str, destination: Option<&Path>) -> anyhow::Result<()> {
    let target = parse_repo_spec(repository)?;
    let api_url = api_url();
    let session_token =
        read_stored_session_token(&api_url)?.context("not signed in; run scope login")?;
    let client = http_client()?;
    let repo_config = get_repo_config(
        &client,
        &api_url,
        &session_token,
        &target.owner,
        &target.repo,
    )?;
    let remote_url = git_remote_url(
        &api_url,
        &permissioned_git_remote_path(&target.owner, &target.repo),
    );
    let checkout_dir = destination
        .map(Path::to_path_buf)
        .unwrap_or_else(|| default_clone_dir(&target.repo));

    clone_with_bearer(&remote_url, &session_token, Some(checkout_dir.as_path()))?;
    install_scope_fetch_auth(&checkout_dir, &remote_url)?;
    write_worktree_scope_repo_config_with_base(&checkout_dir, &repo_config.config)
}

pub fn parse_repo_spec(repository: &str) -> anyhow::Result<RepoSpec> {
    let repository = repository.trim();
    if repository.contains("://") {
        bail!("expected repository as owner/repo");
    }

    let mut parts = repository.split('/');
    let owner = parts.next().unwrap_or_default().trim();
    let repo = parts.next().unwrap_or_default().trim();
    if owner.is_empty() || repo.is_empty() || parts.next().is_some() {
        bail!("expected repository as owner/repo");
    }

    Ok(RepoSpec {
        owner: owner.to_string(),
        repo: repo.to_string(),
    })
}

fn git_remote_url(api_url: &str, git_remote_path: &str) -> String {
    format!(
        "{}/{}",
        api_url.trim_end_matches('/'),
        git_remote_path.trim_start_matches('/')
    )
}

pub fn permissioned_git_remote_path(owner: &str, repo: &str) -> String {
    format!("/git/permissioned/{owner}/{repo}")
}

pub fn default_clone_dir(repo: &str) -> PathBuf {
    repo.strip_suffix(".git")
        .filter(|name| !name.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(repo))
}
