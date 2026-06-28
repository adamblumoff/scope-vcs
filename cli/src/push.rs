use crate::{
    api::{RepoPublicationState, RepoRole, create_clone_credential, get_repo},
    git_repo::{git_remote_push_url, push_head_with_bearer},
};
use anyhow::{Context, bail};
use reqwest::{Url, blocking::Client};

pub const DEFAULT_SCOPE_REMOTE: &str = "scope";
pub const DEFAULT_SCOPE_BRANCH: &str = "main";

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ScopePushTarget {
    pub remote: String,
    pub push_url: String,
    pub owner: String,
    pub repo: String,
}

#[derive(Debug, Eq, PartialEq)]
pub struct ScopePushOutcome {
    pub owner: String,
    pub repo: String,
    pub staged_update_pending: bool,
}

pub fn load_scope_remote(api_url: &str, remote: &str) -> anyhow::Result<ScopePushTarget> {
    let push_url = git_remote_push_url(remote)?;
    let (owner, repo) = parse_scope_git_remote(api_url, &push_url)?;
    let push_url = scope_git_url_without_userinfo(&push_url)?;
    Ok(ScopePushTarget {
        remote: remote.to_string(),
        push_url,
        owner,
        repo,
    })
}

pub fn push_authenticated_remote(
    client: &Client,
    api_url: &str,
    session_token: &str,
    target: &ScopePushTarget,
) -> anyhow::Result<ScopePushOutcome> {
    let repo = get_repo(client, api_url, session_token, &target.owner, &target.repo)?;
    ensure_repo_can_receive_push(
        &target.owner,
        &target.repo,
        repo.lifecycle_state,
        repo.role,
        repo.push_blocked_by_staged_update,
    )?;

    let credential =
        create_clone_credential(client, api_url, session_token, &target.owner, &target.repo)?;
    let git_secret = credential
        .token
        .secret
        .context("API did not return a Git credential")?;
    push_head_with_bearer(&target.push_url, DEFAULT_SCOPE_BRANCH, &git_secret)?;

    let repo = get_repo(client, api_url, session_token, &target.owner, &target.repo)?;
    Ok(ScopePushOutcome {
        owner: repo.owner_handle,
        repo: repo.name,
        staged_update_pending: repo.staged_update_pending,
    })
}

pub fn parse_scope_git_remote(api_url: &str, remote_url: &str) -> anyhow::Result<(String, String)> {
    let api = Url::parse(api_url).context("parse Scope API URL")?;
    let remote = Url::parse(remote_url).context("parse Scope Git remote URL")?;

    if api.scheme() != remote.scheme()
        || api.host_str() != remote.host_str()
        || api.port_or_known_default() != remote.port_or_known_default()
    {
        bail!(
            "Scope remote points at {}, but this CLI is configured for {}",
            redacted_url(&remote),
            api.as_str().trim_end_matches('/')
        );
    }

    if remote.password().is_some() {
        bail!("Scope Git remote URL cannot include a password");
    }

    let segments = remote
        .path_segments()
        .map(|segments| segments.collect::<Vec<_>>())
        .unwrap_or_default();
    if segments.len() != 3 || segments[0] != "git" {
        bail!("Scope remote must have path /git/owner/repo");
    }

    let owner = segments[1].trim();
    let repo = segments[2].trim();
    if owner.is_empty() || repo.is_empty() {
        bail!("Scope remote must have path /git/owner/repo");
    }

    Ok((owner.to_string(), repo.to_string()))
}

fn redacted_url(url: &Url) -> String {
    let mut redacted = url.clone();
    if !redacted.username().is_empty() {
        let _ = redacted.set_username("redacted");
    }
    if redacted.password().is_some() {
        let _ = redacted.set_password(Some("redacted"));
    }
    redacted.to_string()
}

fn ensure_repo_can_receive_push(
    owner: &str,
    repo: &str,
    lifecycle_state: RepoPublicationState,
    role: Option<RepoRole>,
    push_blocked_by_staged_update: bool,
) -> anyhow::Result<()> {
    match lifecycle_state {
        RepoPublicationState::PendingFirstPush => {
            bail!("repo {owner}/{repo} is waiting for its first push. Run: scope init");
        }
        RepoPublicationState::PendingPublish => {
            bail!(
                "repo {owner}/{repo} has a pending review; publish or reject it before pushing again"
            );
        }
        RepoPublicationState::Published => {}
    }

    if push_blocked_by_staged_update {
        bail!(
            "repo {owner}/{repo} has a pending review; publish or reject it before pushing again"
        );
    }

    if role.is_none_or(|role| role < RepoRole::Writer) {
        bail!("you do not have write access to {owner}/{repo}");
    }

    Ok(())
}

fn scope_git_url_without_userinfo(remote_url: &str) -> anyhow::Result<String> {
    let mut url = Url::parse(remote_url).context("parse Scope Git remote URL")?;
    let _ = url.set_username("");
    let _ = url.set_password(None);
    Ok(url.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_scope_git_remote_accepts_matching_scope_remote() {
        assert_eq!(
            parse_scope_git_remote(
                "https://scope-api-production-0251.up.railway.app",
                "https://scope-api-production-0251.up.railway.app/git/adam/random"
            )
            .unwrap(),
            ("adam".to_string(), "random".to_string())
        );
    }

    #[test]
    fn parse_scope_git_remote_accepts_username_without_treating_it_as_auth() {
        assert_eq!(
            parse_scope_git_remote(
                "https://scope.example",
                "https://scope@scope.example/git/adam/random"
            )
            .unwrap(),
            ("adam".to_string(), "random".to_string())
        );
    }

    #[test]
    fn parse_scope_git_remote_rejects_different_origins() {
        assert!(
            parse_scope_git_remote("https://scope.example", "https://evil.example/git/a/b")
                .is_err()
        );
    }

    #[test]
    fn parse_scope_git_remote_rejects_passwords() {
        assert!(
            parse_scope_git_remote(
                "https://scope.example",
                "https://scope:secret@scope.example/git/a/b"
            )
            .is_err()
        );
    }

    #[test]
    fn parse_scope_git_remote_redacts_userinfo_in_mismatch_errors() {
        let error = parse_scope_git_remote(
            "https://scope.example",
            "https://scope:secret@evil.example/git/a/b",
        )
        .unwrap_err()
        .to_string();

        assert!(!error.contains("scope:secret"), "{error}");
        assert!(!error.contains("secret"), "{error}");
        assert!(error.contains("redacted:redacted"), "{error}");
    }

    #[test]
    fn parse_scope_git_remote_rejects_non_scope_paths() {
        for remote in [
            "https://scope.example/adam/random",
            "https://scope.example/git/adam",
            "https://scope.example/git/adam/random/extra",
        ] {
            assert!(parse_scope_git_remote("https://scope.example", remote).is_err());
        }
    }
}
