use anyhow::{Context, bail};
use reqwest::Url;

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
}
