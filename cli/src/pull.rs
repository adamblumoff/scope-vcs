use crate::{
    api::{api_url, http_client},
    git_repo::{
        GitRepo, current_branch, ensure_git_repo_ready, git_remote_fetch_url,
        install_scope_fetch_auth, run_git_in_repo,
    },
    git_transport::{ScopeRemote, select_scope_fetch_remote},
    login::session_from_cache_or_browser,
    push::DEFAULT_SCOPE_BRANCH,
};
use anyhow::{Context, bail};
use std::{collections::BTreeMap, process::Command};

pub fn run(explicit_remote: Option<&str>) -> anyhow::Result<()> {
    let repo = ensure_git_repo_ready("scope pull")?;
    let api_url = api_url();
    let remote = select_scope_fetch_remote(&repo, &api_url, explicit_remote)?;
    let target = ScopeRemote::parse(&api_url, &remote, &git_remote_fetch_url(&repo, &remote)?)?;
    let client = http_client()?;
    let _session = session_from_cache_or_browser(&client, &api_url)?;

    // Persist the permissioned URL and credential helper so plain `git fetch` and
    // `git pull` have exactly the same view after this command returns.
    run_git_in_repo(
        &repo,
        &["remote", "set-url", &remote, &target.permissioned_url],
    )?;
    install_scope_fetch_auth(&repo.root, &target.permissioned_url)?;

    let before = remote_refs(&repo, &remote)?;
    run_git_in_repo(&repo, &["fetch", "--prune", &remote])?;
    let after = remote_refs(&repo, &remote)?;
    print_ref_changes(&remote, &before, &after);

    let branch = current_branch(&repo)?;
    let tracked = format!("refs/remotes/{remote}/{branch}");
    if after.contains_key(&branch) && current_branch_tracks(&repo, &remote, &branch)? {
        run_git_in_repo(&repo, &["merge", "--ff-only", &tracked])?;
        println!("{branch} is up to date with {remote}/{branch}.");
    } else if after.contains_key(&branch) {
        println!(
            "Fetched every visible Scope ref; local branch {branch} does not track {remote}/{branch}, so it was not moved."
        );
    } else {
        println!(
            "Fetched every visible Scope ref; local branch {branch} has no {remote}/{branch} counterpart."
        );
    }
    Ok(())
}

fn current_branch_tracks(repo: &GitRepo, remote: &str, branch: &str) -> anyhow::Result<bool> {
    let output = Command::new("git")
        .current_dir(&repo.root)
        .args([
            "rev-parse",
            "--abbrev-ref",
            "--symbolic-full-name",
            "@{upstream}",
        ])
        .output()
        .context("inspect current branch upstream")?;
    if !output.status.success() {
        return Ok(false);
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim() == format!("{remote}/{branch}"))
}

fn remote_refs(repo: &GitRepo, remote: &str) -> anyhow::Result<BTreeMap<String, String>> {
    let prefix = format!("refs/remotes/{remote}");
    let output = Command::new("git")
        .current_dir(&repo.root)
        .args([
            "for-each-ref",
            "--format=%(refname:strip=3) %(objectname)",
            &prefix,
        ])
        .output()
        .context("inspect Scope remote refs")?;
    if !output.status.success() {
        bail!("inspect Scope remote refs failed");
    }
    Ok(String::from_utf8_lossy(&output.stdout)
        .lines()
        .filter_map(|line| line.split_once(' '))
        .filter(|(name, _)| *name != "HEAD")
        .map(|(name, oid)| (name.to_string(), oid.to_string()))
        .collect())
}

fn print_ref_changes(
    remote: &str,
    before: &BTreeMap<String, String>,
    after: &BTreeMap<String, String>,
) {
    let mut changed = false;
    for (name, oid) in after {
        match before.get(name) {
            None => {
                changed = true;
                let kind = if name == DEFAULT_SCOPE_BRANCH {
                    "branch"
                } else {
                    "request"
                };
                println!("  [new {kind}] {name} -> {remote}/{name}");
            }
            Some(previous) if previous != oid => {
                changed = true;
                println!(
                    "  [updated] {name} {}..{}",
                    short_oid(previous),
                    short_oid(oid)
                );
            }
            _ => {}
        }
    }
    for name in before.keys().filter(|name| !after.contains_key(*name)) {
        changed = true;
        println!("  [removed] {remote}/{name}");
    }
    if !changed {
        println!("No remote refs changed.");
    }
}

fn short_oid(oid: &str) -> &str {
    oid.get(..7).unwrap_or(oid)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::TestDir;
    use std::fs;

    #[test]
    fn short_oid_handles_short_values() {
        assert_eq!(short_oid("0123456789"), "0123456");
        assert_eq!(short_oid("abc"), "abc");
    }

    #[test]
    fn current_branch_must_track_the_selected_remote_before_merge() {
        let dir = TestDir::git_repo("pull-upstream", "main");
        dir.run_git(["config", "user.name", "Scope Test"]);
        dir.run_git(["config", "user.email", "scope@example.invalid"]);
        fs::write(dir.path().join("README.md"), "test\n").unwrap();
        dir.run_git(["add", "README.md"]);
        dir.run_git(["commit", "--quiet", "-m", "initial"]);
        dir.run_git([
            "remote",
            "add",
            "origin",
            "https://example.invalid/repo.git",
        ]);
        dir.run_git(["update-ref", "refs/remotes/origin/main", "HEAD"]);
        dir.run_git(["config", "branch.main.remote", "origin"]);
        dir.run_git(["config", "branch.main.merge", "refs/heads/main"]);
        let repo = GitRepo {
            root: dir.path().to_path_buf(),
        };

        assert!(current_branch_tracks(&repo, "origin", "main").unwrap());
        assert!(!current_branch_tracks(&repo, "scope", "main").unwrap());
    }
}
