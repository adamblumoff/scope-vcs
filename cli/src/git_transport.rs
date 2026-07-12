use crate::git_repo::{GitRepo, git_remote_fetch_url, git_remote_names, git_remote_push_url};
use anyhow::{Context, bail};
use reqwest::Url;

pub const DEFAULT_SCOPE_REMOTE: &str = "scope";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum GitAccess {
    Public,
    Permissioned,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ScopeRemote {
    pub remote: String,
    pub access: GitAccess,
    pub public_url: String,
    pub permissioned_url: String,
    pub owner: String,
    pub repo: String,
}

impl ScopeRemote {
    pub fn parse(api_url: &str, name: &str, remote_url: &str) -> anyhow::Result<Self> {
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
        if segments.len() != 4 || segments[0] != "git" {
            bail!(
                "Scope remote must have path /git/public/owner/repo or /git/permissioned/owner/repo"
            );
        }
        let access = match segments[1] {
            "public" => GitAccess::Public,
            "permissioned" => GitAccess::Permissioned,
            _ => bail!(
                "Scope remote must have path /git/public/owner/repo or /git/permissioned/owner/repo"
            ),
        };
        let owner = segments[2].trim();
        let repo = segments[3].trim();
        if owner.is_empty() || repo.is_empty() {
            bail!("Scope remote must include owner and repo");
        }

        Ok(Self {
            remote: name.to_string(),
            access,
            public_url: url_for_access(&remote, GitAccess::Public, owner, repo),
            permissioned_url: url_for_access(&remote, GitAccess::Permissioned, owner, repo),
            owner: owner.to_string(),
            repo: repo.to_string(),
        })
    }
}

pub fn select_scope_fetch_remote(
    repo: &GitRepo,
    api_url: &str,
    explicit_remote: Option<&str>,
) -> anyhow::Result<String> {
    if let Some(remote) = normalized_remote(explicit_remote) {
        ScopeRemote::parse(api_url, &remote, &git_remote_fetch_url(repo, &remote)?)?;
        return Ok(remote);
    }

    for candidate in scope_remote_candidates(repo)? {
        let Ok(url) = git_remote_fetch_url(repo, &candidate) else {
            continue;
        };
        if ScopeRemote::parse(api_url, &candidate, &url).is_ok() {
            return Ok(candidate);
        }
    }
    bail!("no Scope Git remote found; pass --remote <name> or run scope init")
}

pub fn select_scope_push_remote(
    repo: &GitRepo,
    api_url: &str,
    explicit_remote: Option<&str>,
) -> anyhow::Result<String> {
    if let Some(remote) = normalized_remote(explicit_remote) {
        ScopeRemote::parse(api_url, &remote, &git_remote_push_url(repo, &remote)?)?;
        return Ok(remote);
    }

    for candidate in scope_remote_candidates(repo)? {
        let Ok(push_url) = git_remote_push_url(repo, &candidate) else {
            continue;
        };
        let Ok(push_target) = ScopeRemote::parse(api_url, &candidate, &push_url) else {
            continue;
        };
        if push_target.access != GitAccess::Permissioned {
            continue;
        }
        let Ok(fetch_url) = git_remote_fetch_url(repo, &candidate) else {
            continue;
        };
        let Ok(fetch_target) = ScopeRemote::parse(api_url, &candidate, &fetch_url) else {
            continue;
        };
        if fetch_target.owner == push_target.owner && fetch_target.repo == push_target.repo {
            return Ok(candidate);
        }
    }
    bail!("no Scope Git remote found; pass --remote <name> or run scope init")
}

fn normalized_remote(remote: Option<&str>) -> Option<String> {
    remote
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

fn scope_remote_candidates(repo: &GitRepo) -> anyhow::Result<Vec<String>> {
    let mut remotes = git_remote_names(repo)?;
    remotes.sort_by_key(|remote| match remote.as_str() {
        DEFAULT_SCOPE_REMOTE => 0,
        "origin" => 1,
        _ => 2,
    });
    Ok(remotes)
}

fn url_for_access(remote: &Url, access: GitAccess, owner: &str, repo: &str) -> String {
    let mut url = remote.clone();
    let _ = url.set_username("");
    let _ = url.set_password(None);
    let mode = match access {
        GitAccess::Public => "public",
        GitAccess::Permissioned => "permissioned",
    };
    url.set_path(&format!("/git/{mode}/{owner}/{repo}"));
    url.set_query(None);
    url.set_fragment(None);
    url.to_string()
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::TestDir;

    #[test]
    fn parses_scope_remote_once_and_derives_both_access_urls() {
        let remote = ScopeRemote::parse(
            "https://scope.example",
            "origin",
            "https://scope@scope.example/git/public/adam/repo?ignored=true",
        )
        .unwrap();

        assert_eq!(remote.remote, "origin");
        assert_eq!(remote.access, GitAccess::Public);
        assert_eq!(remote.owner, "adam");
        assert_eq!(remote.repo, "repo");
        assert_eq!(
            remote.public_url,
            "https://scope.example/git/public/adam/repo"
        );
        assert_eq!(
            remote.permissioned_url,
            "https://scope.example/git/permissioned/adam/repo"
        );
    }

    #[test]
    fn mismatch_errors_redact_remote_credentials() {
        let error = ScopeRemote::parse(
            "https://scope.example",
            "origin",
            "https://scope:secret@evil.example/git/public/adam/repo",
        )
        .unwrap_err()
        .to_string();

        assert!(!error.contains("secret"), "{error}");
        assert!(error.contains("redacted:redacted"), "{error}");
    }

    #[test]
    fn rejects_passwords_and_non_scope_paths() {
        for remote in [
            "https://scope:secret@scope.example/git/permissioned/adam/repo",
            "https://scope.example/adam/repo",
            "https://scope.example/git/private/adam/repo",
            "https://scope.example/git/public/adam",
            "https://scope.example/git/public/adam/repo/extra",
        ] {
            assert!(
                ScopeRemote::parse("https://scope.example", "origin", remote).is_err(),
                "accepted {remote}"
            );
        }
    }

    #[test]
    fn discovers_scope_remote_by_conventional_name_then_url() {
        let dir = TestDir::git_repo("scope-remote-discovery", "main");
        dir.run_git([
            "remote",
            "add",
            "origin",
            "https://scope.example/git/permissioned/adam/repo",
        ]);
        let repo = GitRepo {
            root: dir.path().to_path_buf(),
        };

        assert_eq!(
            select_scope_fetch_remote(&repo, "https://scope.example", None).unwrap(),
            "origin"
        );

        dir.run_git([
            "remote",
            "add",
            "scope",
            "https://scope.example/git/permissioned/adam/repo",
        ]);
        assert_eq!(
            select_scope_fetch_remote(&repo, "https://scope.example", None).unwrap(),
            "scope"
        );
    }

    #[test]
    fn explicit_push_remote_uses_push_url() {
        let dir = TestDir::git_repo("scope-push-remote-discovery", "main");
        dir.run_git(["remote", "add", "origin", "https://github.com/adam/repo"]);
        dir.run_git([
            "remote",
            "set-url",
            "--push",
            "origin",
            "https://scope.example/git/permissioned/adam/repo",
        ]);
        let repo = GitRepo {
            root: dir.path().to_path_buf(),
        };

        assert!(select_scope_push_remote(&repo, "https://scope.example", None).is_err());
        assert_eq!(
            select_scope_push_remote(&repo, "https://scope.example", Some("origin")).unwrap(),
            "origin"
        );
    }

    #[test]
    fn push_discovery_skips_public_and_mismatched_fetch_remotes() {
        let dir = TestDir::git_repo("scope-push-safe-discovery", "main");
        dir.run_git([
            "remote",
            "add",
            "origin",
            "https://scope.example/git/public/adam/repo",
        ]);
        dir.run_git(["remote", "add", "github", "https://github.com/adam/repo"]);
        dir.run_git([
            "remote",
            "set-url",
            "--push",
            "github",
            "https://scope.example/git/permissioned/adam/repo",
        ]);
        dir.run_git([
            "remote",
            "add",
            "upstream",
            "https://scope.example/git/permissioned/adam/repo",
        ]);
        let repo = GitRepo {
            root: dir.path().to_path_buf(),
        };

        assert_eq!(
            select_scope_push_remote(&repo, "https://scope.example", None).unwrap(),
            "upstream"
        );
    }
}
