use crate::{
    api::{api_url, create_clone_credential, http_client},
    auth::read_stored_session_token,
    git_credentials::clone_with_credential,
};
use anyhow::{Context, bail};
use std::path::Path;

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
    let credential = create_clone_credential(
        &client,
        &api_url,
        &session_token,
        &target.owner,
        &target.repo,
    )?;
    let secret = credential
        .token
        .secret
        .context("API did not return a Git clone token")?;
    let remote_url = git_remote_url(&api_url, &credential.git_remote_path);

    clone_with_credential(&remote_url, &secret, destination)
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
