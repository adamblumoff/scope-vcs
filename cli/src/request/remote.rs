use crate::{
    git_repo::{GitRepo, branch_config_value, current_branch, git_remote_fetch_url},
    git_transport::{ScopeRemote, select_scope_fetch_remote},
};

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
    select_scope_fetch_remote(git_repo, api_url, None)
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::TestDir as TempDir;
    use std::{fs, path::PathBuf};

    #[test]
    fn selected_remote_prefers_explicit_then_branch_remote() {
        for (explicit, branch, expected) in [
            (
                Some("upstream"),
                Some("scope".to_string()),
                Some("upstream"),
            ),
            (None, Some("upstream".to_string()), Some("upstream")),
            (None, None, None),
            (Some(" "), Some(" ".to_string()), None),
        ] {
            assert_eq!(selected_remote_name(explicit, branch).as_deref(), expected);
        }
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
    fn default_remote_discovers_scope_remotes_in_priority_order() {
        for (label, remotes, expected) in [
            (
                "origin",
                vec![("origin", "https://scope.example/git/public/adam/repo")],
                "origin",
            ),
            (
                "scope-first",
                vec![
                    ("origin", "https://scope.example/git/public/adam/repo"),
                    ("scope", "https://scope.example/git/permissioned/adam/repo"),
                ],
                "scope",
            ),
            (
                "renamed",
                vec![
                    ("origin", "https://github.com/adam/repo"),
                    ("upstream", "https://scope.example/git/public/adam/repo"),
                ],
                "upstream",
            ),
        ] {
            let (_dir, repo) = repo_with_remotes(label, &remotes);
            assert_eq!(
                request_remote_name(&repo, "https://scope.example", None).unwrap(),
                expected
            );
        }
    }

    fn repo_with_remotes(label: &str, remotes: &[(&str, &str)]) -> (TempDir, GitRepo) {
        let dir = TempDir::new(label);
        dir.run_git(["-c", "init.defaultBranch=main", "init"]);
        fs::write(dir.path().join("README.md"), "# sample\n").unwrap();
        dir.run_git(["add", "README.md"]);
        dir.run_git([
            "-c",
            "user.name=Scope Tests",
            "-c",
            "user.email=scope-tests@example.com",
            "commit",
            "-m",
            "initial commit",
        ]);
        for (remote, url) in remotes {
            dir.run_git(["remote", "add", remote, url]);
        }
        let repo = GitRepo {
            root: dir.path().to_path_buf(),
        };
        (dir, repo)
    }
}
