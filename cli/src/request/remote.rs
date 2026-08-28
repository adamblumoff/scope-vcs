use crate::{
    git_repo::{
        GitRepo, branch_config_value, current_branch, git_remote_fetch_url, git_remote_names,
        scope_git_origin,
    },
    git_transport::{DEFAULT_SCOPE_REMOTE, ScopeRemote},
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
    if let Some(remote) = normalized_remote_arg(
        branch_config_value(git_repo, &branch, REQUEST_REMOTE_KEY)?.as_deref(),
    ) {
        return Ok(remote);
    }
    if let Some(remote) =
        normalized_remote_arg(branch_config_value(git_repo, &branch, "remote")?.as_deref())
        && load_request_remote(git_repo, api_url, &remote).is_ok()
    {
        return Ok(remote);
    }
    unambiguous_scope_remote(git_repo, api_url)
}

fn unambiguous_scope_remote(git_repo: &GitRepo, api_url: &str) -> anyhow::Result<String> {
    let git_origin = scope_git_origin(git_repo, api_url)?;
    let mut targets = git_remote_names(git_repo)?
        .into_iter()
        .filter_map(|remote| {
            let url = git_remote_fetch_url(git_repo, &remote).ok()?;
            ScopeRemote::parse(&git_origin, &remote, &url).ok()
        })
        .collect::<Vec<_>>();
    if targets.is_empty() {
        bail!("no Scope Git remote found; pass --remote <name> or run scope init");
    }
    let first_target = (&targets[0].owner, &targets[0].repo);
    if targets
        .iter()
        .any(|target| (&target.owner, &target.repo) != first_target)
    {
        bail!("multiple Scope repositories are configured; pass --remote <name> to choose one");
    }
    targets.sort_by_key(|target| match target.remote.as_str() {
        DEFAULT_SCOPE_REMOTE => 0,
        "origin" => 1,
        _ => 2,
    });
    Ok(targets.remove(0).remote)
}

pub(super) fn load_request_remote(
    git_repo: &GitRepo,
    api_url: &str,
    remote: &str,
) -> anyhow::Result<RequestRemoteTarget> {
    let fetch_url = git_remote_fetch_url(git_repo, remote)?;
    ScopeRemote::parse(&scope_git_origin(git_repo, api_url)?, remote, &fetch_url)
}

fn normalized_remote_arg(remote: Option<&str>) -> Option<String> {
    remote
        .map(|remote| remote.trim().to_string())
        .filter(|remote| !remote.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::TestDir as TempDir;
    use std::{fs, path::PathBuf};

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
    fn implicit_remote_selection_follows_the_priority_matrix() {
        for (label, remotes, tracking_remote, expected) in [
            (
                "origin",
                vec![("origin", "https://scope.example/git/public/adam/repo")],
                None,
                "origin",
            ),
            (
                "scope-first",
                vec![
                    ("origin", "https://scope.example/git/public/adam/repo"),
                    ("scope", "https://scope.example/git/permissioned/adam/repo"),
                ],
                None,
                "scope",
            ),
            (
                "renamed",
                vec![
                    ("origin", "https://github.com/adam/repo"),
                    ("upstream", "https://scope.example/git/public/adam/repo"),
                ],
                None,
                "upstream",
            ),
            (
                "non-scope-tracking-falls-back",
                vec![
                    ("origin", "https://github.com/adam/repo"),
                    ("scope", "https://scope.example/git/public/adam/repo"),
                ],
                Some("origin"),
                "scope",
            ),
            (
                "scope-tracking-wins-over-ambiguity",
                vec![
                    ("origin", "https://scope.example/git/public/adam/one"),
                    ("scope", "https://scope.example/git/public/adam/two"),
                ],
                Some("origin"),
                "origin",
            ),
        ] {
            let (dir, repo) = repo_with_remotes(label, &remotes);
            if let Some(remote) = tracking_remote {
                dir.run_git(["config", "branch.main.remote", remote]);
            }
            assert_eq!(
                request_remote_name(&repo, "https://scope.example", None).unwrap(),
                expected,
                "{label}"
            );
        }
    }

    #[test]
    fn distinct_scope_repository_targets_require_an_explicit_remote() {
        let (_dir, repo) = repo_with_remotes(
            "ambiguous",
            &[
                ("origin", "https://scope.example/git/public/adam/one"),
                ("scope", "https://scope.example/git/public/adam/two"),
            ],
        );

        let error = request_remote_name(&repo, "https://scope.example", None)
            .unwrap_err()
            .to_string();
        assert!(error.contains("pass --remote"), "{error}");
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
