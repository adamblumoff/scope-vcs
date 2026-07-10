use crate::{
    git_repo::{
        GitRepo, branch_config_value, current_branch, git_remote_fetch_url, git_remote_names,
    },
    git_transport::ScopeRemote,
    push::DEFAULT_SCOPE_REMOTE,
};
use anyhow::bail;

pub(super) const REQUEST_REMOTE_KEY: &str = "scopeRequestRemote";

pub(super) type RequestRemoteTarget = ScopeRemote;

pub(super) fn request_remote_name(
    git_repo: &GitRepo,
    api_url: &str,
    explicit_remote: Option<&str>,
) -> anyhow::Result<String> {
    if let Some(remote) = normalized_remote_arg(explicit_remote) {
        return Ok(remote);
    }
    let branch = current_branch(git_repo)?;
    if let Some(remote) = selected_remote_name(
        None,
        branch_config_value(git_repo, &branch, REQUEST_REMOTE_KEY)?,
    ) {
        return Ok(remote);
    }
    default_request_remote_name(git_repo, api_url)
}

pub(super) fn load_request_remote(
    git_repo: &GitRepo,
    api_url: &str,
    remote: &str,
) -> anyhow::Result<RequestRemoteTarget> {
    let fetch_url = git_remote_fetch_url(git_repo, remote)?;
    ScopeRemote::parse(api_url, remote, &fetch_url)
}

fn normalized_remote_arg(remote: Option<&str>) -> Option<String> {
    remote
        .map(|remote| remote.trim().to_string())
        .filter(|remote| !remote.is_empty())
}

fn selected_remote_name(
    explicit_remote: Option<&str>,
    branch_remote: Option<String>,
) -> Option<String> {
    normalized_remote_arg(explicit_remote)
        .or_else(|| normalized_remote_arg(branch_remote.as_deref()))
}

fn default_request_remote_name(git_repo: &GitRepo, api_url: &str) -> anyhow::Result<String> {
    let remotes = git_remote_names(git_repo)?;
    for candidate in [DEFAULT_SCOPE_REMOTE, "origin"] {
        if remotes.iter().any(|remote| remote == candidate)
            && remote_points_to_scope_repo(git_repo, api_url, candidate)
        {
            return Ok(candidate.to_string());
        }
    }
    for remote in remotes {
        if remote_points_to_scope_repo(git_repo, api_url, &remote) {
            return Ok(remote);
        }
    }
    bail!("no Scope Git remote found; pass --remote <name> or run scope init")
}

fn remote_points_to_scope_repo(git_repo: &GitRepo, api_url: &str, remote: &str) -> bool {
    git_remote_fetch_url(git_repo, remote)
        .and_then(|remote_url| ScopeRemote::parse(api_url, remote, &remote_url).map(|_| ()))
        .is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::TestDir as TempDir;
    use std::{
        fs,
        path::{Path, PathBuf},
        process::{Command, Output},
    };

    #[test]
    fn request_remote_accepts_public_git_remote_and_derives_permissioned_push_url() {
        let target = ScopeRemote::parse(
            "https://scope.example",
            "origin",
            "https://scope.example/git/public/adam/repo",
        )
        .unwrap();

        assert_eq!(target.owner, "adam");
        assert_eq!(target.repo, "repo");
        assert_eq!(
            target.public_url,
            "https://scope.example/git/public/adam/repo"
        );
        assert_eq!(
            target.permissioned_url,
            "https://scope.example/git/permissioned/adam/repo"
        );
    }

    #[test]
    fn request_remote_accepts_permissioned_git_remote_and_derives_public_fetch_url() {
        let target = ScopeRemote::parse(
            "https://scope.example",
            "scope",
            "https://scope@scope.example/git/permissioned/adam/repo",
        )
        .unwrap();

        assert_eq!(
            target.public_url,
            "https://scope.example/git/public/adam/repo"
        );
        assert_eq!(
            target.permissioned_url,
            "https://scope.example/git/permissioned/adam/repo"
        );
    }

    #[test]
    fn selected_remote_prefers_explicit_remote() {
        assert_eq!(
            selected_remote_name(Some("upstream"), Some("scope".to_string())),
            Some("upstream".to_string())
        );
    }

    #[test]
    fn selected_remote_uses_branch_remote_before_default() {
        assert_eq!(
            selected_remote_name(None, Some("upstream".to_string())),
            Some("upstream".to_string())
        );
    }

    #[test]
    fn selected_remote_returns_none_without_explicit_or_branch_remote() {
        assert_eq!(selected_remote_name(None, None), None);
        assert_eq!(selected_remote_name(Some(" "), Some(" ".to_string())), None);
    }

    #[test]
    fn explicit_remote_does_not_require_git_branch_context() {
        let repo = GitRepo {
            root: PathBuf::from("/does/not/need/to/exist"),
        };

        assert_eq!(
            request_remote_name(&repo, "https://scope.example", Some("origin")).unwrap(),
            "origin"
        );
    }

    #[test]
    fn default_remote_discovers_origin_scope_remote() {
        let (_dir, repo) = repo_with_remotes(
            "origin",
            &[("origin", "https://scope.example/git/public/adam/repo")],
        );

        assert_eq!(
            request_remote_name(&repo, "https://scope.example", None).unwrap(),
            "origin"
        );
    }

    #[test]
    fn default_remote_prefers_scope_before_origin() {
        let (_dir, repo) = repo_with_remotes(
            "scope-first",
            &[
                ("origin", "https://scope.example/git/public/adam/repo"),
                ("scope", "https://scope.example/git/permissioned/adam/repo"),
            ],
        );

        assert_eq!(
            request_remote_name(&repo, "https://scope.example", None).unwrap(),
            DEFAULT_SCOPE_REMOTE
        );
    }

    #[test]
    fn default_remote_finds_nonstandard_scope_remote() {
        let (_dir, repo) = repo_with_remotes(
            "renamed",
            &[
                ("origin", "https://github.com/adam/repo"),
                ("upstream", "https://scope.example/git/public/adam/repo"),
            ],
        );

        assert_eq!(
            request_remote_name(&repo, "https://scope.example", None).unwrap(),
            "upstream"
        );
    }

    fn repo_with_remotes(label: &str, remotes: &[(&str, &str)]) -> (TempDir, GitRepo) {
        let dir = TempDir::new(label);
        run_git(dir.path(), ["-c", "init.defaultBranch=main", "init"]);
        fs::write(dir.path().join("README.md"), "# sample\n").unwrap();
        run_git(dir.path(), ["add", "README.md"]);
        run_git(
            dir.path(),
            [
                "-c",
                "user.name=Scope Tests",
                "-c",
                "user.email=scope-tests@example.com",
                "commit",
                "-m",
                "initial commit",
            ],
        );
        for (remote, url) in remotes {
            run_git(dir.path(), ["remote", "add", remote, url]);
        }
        let repo = GitRepo {
            root: dir.path().to_path_buf(),
        };
        (dir, repo)
    }

    fn run_git<const N: usize>(path: &Path, args: [&str; N]) {
        let output = Command::new("git")
            .current_dir(path)
            .args(args)
            .output()
            .unwrap();
        assert_success(&output, "git command");
    }

    fn assert_success(output: &Output, action: &str) {
        assert!(
            output.status.success(),
            "{action} failed\nstdout:\n{}\nstderr:\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }
}
