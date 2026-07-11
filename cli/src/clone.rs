use crate::{
    api::{api_url, get_repo_config, http_client},
    auth::read_stored_session_token,
    git_repo::{clone_with_bearer, install_scope_fetch_auth},
    repo_config::write_worktree_scope_repo_config_with_base,
};
use anyhow::{Context, bail};
use scope_core::domain::repo_config::RepoConfig;
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

    clone_and_configure(
        &remote_url,
        &session_token,
        &checkout_dir,
        &repo_config.config,
    )
}

fn clone_and_configure(
    remote_url: &str,
    session_token: &str,
    checkout_dir: &Path,
    config: &RepoConfig,
) -> anyhow::Result<()> {
    clone_with_bearer(remote_url, session_token, Some(checkout_dir))?;
    install_scope_fetch_auth(checkout_dir, remote_url)?;
    write_worktree_scope_repo_config_with_base(checkout_dir, config)
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        repo_config::{default_scope_repo_config, load_worktree_scope_repo_config},
        test_support::TestDir,
    };
    use std::{fs, process::Command};

    #[test]
    fn clone_installs_fetch_auth_and_repo_config() {
        let dir = TestDir::git_repo("clone-orchestration", "main");
        fs::write(dir.path().join("README.md"), "initial\n").unwrap();
        dir.run_git(["add", "README.md"]);
        dir.run_git([
            "-c",
            "user.name=Scope Test",
            "-c",
            "user.email=scope@example.test",
            "commit",
            "--quiet",
            "-m",
            "initial",
        ]);
        let checkout = dir.path().join("checkout");
        let remote_url = format!("file://{}", dir.path().display());
        let config = default_scope_repo_config();

        clone_and_configure(&remote_url, "secret", &checkout, &config).unwrap();

        assert_eq!(load_worktree_scope_repo_config(&checkout).unwrap(), config);
        let helper = Command::new("git")
            .current_dir(&checkout)
            .args([
                "config",
                "--local",
                "--get-urlmatch",
                "credential.helper",
                &remote_url,
            ])
            .output()
            .unwrap();
        assert_eq!(
            String::from_utf8_lossy(&helper.stdout).trim(),
            "!scope git-credential"
        );
    }
}
