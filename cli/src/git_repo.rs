use anyhow::{Context, bail};
use std::{
    collections::BTreeSet,
    env,
    path::PathBuf,
    process::{Command, Output},
};

#[derive(Debug)]
pub struct GitRepo {
    pub root: PathBuf,
}

#[derive(Debug, Eq, PartialEq)]
pub struct GitPushAuthPlan {
    pub args: Vec<String>,
    pub env: Vec<(String, String)>,
}

#[derive(Debug, Eq, PartialEq)]
pub struct GitFetchAuthPlan {
    pub args: Vec<String>,
    pub env: Vec<(String, String)>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GitChangedPath {
    pub status: String,
    pub path: String,
}

pub fn discover_git_repo(command_name: &str) -> anyhow::Result<GitRepo> {
    let root_output = Command::new("git")
        .args(["rev-parse", "--show-toplevel"])
        .output()
        .context("inspect Git repository")?;
    if !root_output.status.success() {
        bail!("run {command_name} from inside an existing Git repository");
    }

    let root = String::from_utf8_lossy(&root_output.stdout)
        .trim()
        .to_string();
    if root.is_empty() {
        bail!("Git repository root could not be determined");
    }

    Ok(GitRepo {
        root: PathBuf::from(root),
    })
}

pub fn ensure_git_repo_ready(command_name: &str) -> anyhow::Result<GitRepo> {
    let repo = discover_git_repo(command_name)?;
    if !git_success_in_repo(&repo, &["rev-parse", "--verify", "HEAD"]) {
        bail!("create at least one Git commit before running {command_name}");
    }

    Ok(repo)
}

pub fn warn_if_dirty_working_tree(repo: &GitRepo) -> anyhow::Result<()> {
    let output = Command::new("git")
        .current_dir(&repo.root)
        .args(["status", "--porcelain", "--untracked-files=all"])
        .output()
        .context("inspect Git working tree")?;
    if !output.status.success() {
        bail!("git status --porcelain failed");
    }
    if has_dirty_paths_outside_scope_config(&output.stdout) {
        eprintln!("Working tree has uncommitted changes.");
        eprintln!("Only committed HEAD will be pushed to Scope.");
    }
    Ok(())
}

fn has_dirty_paths_outside_scope_config(status: &[u8]) -> bool {
    String::from_utf8_lossy(status).lines().any(|line| {
        let path = line.get(3..).unwrap_or_default();
        path != ".scope/repo.json" && path != ".scope/repo-state.json"
    })
}

pub fn changed_paths_since_last_scope_push(
    repo: &GitRepo,
    remote: &str,
    branch: &str,
) -> anyhow::Result<Vec<GitChangedPath>> {
    changed_paths_since_last_scope_push_at_commit(repo, remote, branch, "HEAD")
}

pub fn changed_paths_since_last_scope_push_at_commit(
    repo: &GitRepo,
    remote: &str,
    branch: &str,
    commit_oid: &str,
) -> anyhow::Result<Vec<GitChangedPath>> {
    let remote_ref = format!("refs/remotes/{remote}/{branch}");
    if git_success_in_repo(repo, &["show-ref", "--verify", "--quiet", &remote_ref]) {
        changed_paths_since_scope_base_at_commit(repo, Some(&remote_ref), commit_oid)
    } else {
        changed_paths_since_scope_base_at_commit(repo, None, commit_oid)
    }
}

pub fn changed_paths_since_scope_base_at_commit(
    repo: &GitRepo,
    base_oid_or_ref: Option<&str>,
    commit_oid: &str,
) -> anyhow::Result<Vec<GitChangedPath>> {
    match base_oid_or_ref {
        Some(base) => {
            let output = git_output_in_repo(
                repo,
                &["diff", "--name-status", &format!("{base}..{commit_oid}")],
            )?;
            if !output.status.success() {
                bail!("inspect committed changes for Scope push review failed");
            }

            Ok(parse_name_status(&output.stdout))
        }
        None => {
            let output = git_output_in_repo(repo, &["ls-tree", "-r", "--name-only", commit_oid])?;
            if !output.status.success() {
                bail!("inspect committed files for Scope first push review failed");
            }

            Ok(parse_tree_paths_as_added(&output.stdout))
        }
    }
}

pub fn worktree_file_paths(repo: &GitRepo) -> anyhow::Result<Vec<String>> {
    let output = git_output_in_repo(
        repo,
        &[
            "ls-files",
            "-z",
            "--cached",
            "--others",
            "--exclude-standard",
        ],
    )?;
    if !output.status.success() {
        bail!("inspect Git worktree files failed");
    }

    let deleted_output = git_output_in_repo(repo, &["ls-files", "-z", "--deleted"])?;
    if !deleted_output.status.success() {
        bail!("inspect deleted Git worktree files failed");
    }

    Ok(exclude_deleted_paths(
        parse_nul_paths(&output.stdout),
        parse_nul_paths(&deleted_output.stdout),
    ))
}

pub fn committed_file_paths_at_commit(
    repo: &GitRepo,
    commit_oid: &str,
) -> anyhow::Result<Vec<String>> {
    let output = git_output_in_repo(repo, &["ls-tree", "-rz", "--name-only", commit_oid])?;
    if !output.status.success() {
        bail!("inspect committed files for Scope review failed");
    }

    Ok(parse_nul_paths(&output.stdout))
}

pub fn scope_remote_head_oid(
    repo: &GitRepo,
    remote: &str,
    branch: &str,
) -> anyhow::Result<Option<String>> {
    let remote_ref = format!("refs/remotes/{remote}/{branch}");
    if !git_success_in_repo(repo, &["show-ref", "--verify", "--quiet", &remote_ref]) {
        return Ok(None);
    }

    let output = git_output_in_repo(repo, &["show-ref", "--hash", "--verify", &remote_ref])?;
    if !output.status.success() {
        bail!("inspect Scope remote ref failed");
    }

    let oid = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if oid.is_empty() {
        Ok(None)
    } else {
        Ok(Some(oid))
    }
}

pub fn mark_scope_remote_pushed(
    repo: &GitRepo,
    remote: &str,
    branch: &str,
    commit_oid: &str,
) -> anyhow::Result<()> {
    let remote_ref = format!("refs/remotes/{remote}/{branch}");
    let status = Command::new("git")
        .current_dir(&repo.root)
        .args(["update-ref", &remote_ref, commit_oid])
        .status()
        .with_context(|| format!("mark {remote_ref} as pushed"))?;
    if !status.success() {
        bail!("git update-ref {remote_ref} {commit_oid} failed");
    }
    Ok(())
}

pub fn git_remote_push_url(remote: &str) -> anyhow::Result<String> {
    let output = git_output(&["remote", "get-url", "--push", remote])?;
    if !output.status.success() {
        bail!("Scope remote '{remote}' is not configured. Run: scope init");
    }

    let url = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if url.is_empty() {
        bail!("Scope remote '{remote}' has an empty push URL");
    }
    Ok(url)
}

pub fn run_git(args: &[&str]) -> anyhow::Result<()> {
    let status = Command::new("git")
        .args(args)
        .status()
        .with_context(|| format!("run git {}", args.join(" ")))?;
    if !status.success() {
        bail!("git {} failed", args.join(" "));
    }
    Ok(())
}

pub fn push_head_with_bearer(
    destination: &str,
    commit_oid: &str,
    branch: &str,
    bearer_token: &str,
    push_intent_token: &str,
) -> anyhow::Result<()> {
    let inherited_config_count = env::var("GIT_CONFIG_COUNT")
        .ok()
        .and_then(|value| value.parse::<usize>().ok());
    let plan = git_push_auth_plan(
        destination,
        commit_oid,
        branch,
        bearer_token,
        push_intent_token,
        inherited_config_count,
    );
    let mut command = Command::new("git");
    command.args(plan.args.iter().map(String::as_str));
    for (key, value) in plan.env {
        command.env(key, value);
    }

    let status = command
        .status()
        .context("run authenticated Scope git push")?;
    if !status.success() {
        bail!("git push to Scope failed");
    }
    Ok(())
}

pub fn fetch_scope_remote_with_bearer(
    repo: &GitRepo,
    destination: &str,
    remote: &str,
    branch: &str,
    bearer_token: &str,
) -> anyhow::Result<()> {
    let inherited_config_count = env::var("GIT_CONFIG_COUNT")
        .ok()
        .and_then(|value| value.parse::<usize>().ok());
    let plan = git_fetch_auth_plan(
        destination,
        remote,
        branch,
        bearer_token,
        inherited_config_count,
    );
    let mut command = Command::new("git");
    command.current_dir(&repo.root);
    command.args(plan.args.iter().map(String::as_str));
    for (key, value) in plan.env {
        command.env(key, value);
    }

    let status = command
        .status()
        .context("refresh Scope Git remote before push review")?;
    if !status.success() {
        bail!("refresh Scope Git remote before push review failed");
    }
    Ok(())
}

pub fn git_push_auth_plan(
    destination: &str,
    commit_oid: &str,
    branch: &str,
    bearer_token: &str,
    push_intent_token: &str,
    inherited_config_count: Option<usize>,
) -> GitPushAuthPlan {
    let config_index = inherited_config_count.unwrap_or(0);
    let push_intent_header_config_index = config_index + 1;
    GitPushAuthPlan {
        args: vec![
            "-c".to_string(),
            "push.recurseSubmodules=no".to_string(),
            "push".to_string(),
            destination.to_string(),
            format!("{commit_oid}:refs/heads/{branch}"),
        ],
        env: vec![
            (
                "GIT_CONFIG_COUNT".to_string(),
                (config_index + 2).to_string(),
            ),
            (
                format!("GIT_CONFIG_KEY_{config_index}"),
                format!("http.{destination}.extraHeader"),
            ),
            (
                format!("GIT_CONFIG_VALUE_{config_index}"),
                format!("Authorization: Bearer {bearer_token}"),
            ),
            (
                format!("GIT_CONFIG_KEY_{push_intent_header_config_index}"),
                format!("http.{destination}.extraHeader"),
            ),
            (
                format!("GIT_CONFIG_VALUE_{push_intent_header_config_index}"),
                format!("X-Scope-Push-Intent: {push_intent_token}"),
            ),
        ],
    }
}

pub fn git_fetch_auth_plan(
    destination: &str,
    remote: &str,
    branch: &str,
    bearer_token: &str,
    inherited_config_count: Option<usize>,
) -> GitFetchAuthPlan {
    let config_index = inherited_config_count.unwrap_or(0);
    GitFetchAuthPlan {
        args: vec![
            "-c".to_string(),
            "protocol.version=2".to_string(),
            "fetch".to_string(),
            "--no-tags".to_string(),
            destination.to_string(),
            format!("+refs/heads/{branch}:refs/remotes/{remote}/{branch}"),
        ],
        env: vec![
            (
                "GIT_CONFIG_COUNT".to_string(),
                (config_index + 1).to_string(),
            ),
            (
                format!("GIT_CONFIG_KEY_{config_index}"),
                format!("http.{destination}.extraHeader"),
            ),
            (
                format!("GIT_CONFIG_VALUE_{config_index}"),
                format!("Authorization: Bearer {bearer_token}"),
            ),
        ],
    }
}

pub fn head_oid(repo: &GitRepo) -> anyhow::Result<String> {
    let output = git_output_in_repo(repo, &["rev-parse", "HEAD"])?;
    if !output.status.success() {
        bail!("inspect Git HEAD failed");
    }

    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

fn git_output(args: &[&str]) -> anyhow::Result<Output> {
    Command::new("git")
        .args(args)
        .output()
        .with_context(|| format!("run git {}", args.join(" ")))
}

fn git_output_in_repo(repo: &GitRepo, args: &[&str]) -> anyhow::Result<Output> {
    Command::new("git")
        .current_dir(&repo.root)
        .args(args)
        .output()
        .with_context(|| format!("run git {}", args.join(" ")))
}

fn git_success_in_repo(repo: &GitRepo, args: &[&str]) -> bool {
    Command::new("git")
        .current_dir(&repo.root)
        .args(args)
        .status()
        .is_ok_and(|status| status.success())
}

fn parse_name_status(output: &[u8]) -> Vec<GitChangedPath> {
    String::from_utf8_lossy(output)
        .lines()
        .filter_map(|line| {
            let mut parts = line.split('\t');
            let status = parts.next()?.trim();
            let path = parts.next()?.trim();
            if status.is_empty() || path.is_empty() {
                return None;
            }
            let path = match parts.next() {
                Some(next_path) => format!("{path} -> {}", next_path.trim()),
                None => path.to_string(),
            };
            Some(GitChangedPath {
                status: status.to_string(),
                path,
            })
        })
        .collect()
}

fn parse_tree_paths_as_added(output: &[u8]) -> Vec<GitChangedPath> {
    String::from_utf8_lossy(output)
        .lines()
        .map(str::trim)
        .filter(|path| !path.is_empty())
        .map(|path| GitChangedPath {
            status: "A".to_string(),
            path: path.to_string(),
        })
        .collect()
}

fn parse_nul_paths(output: &[u8]) -> Vec<String> {
    output
        .split(|byte| *byte == 0)
        .filter_map(|path| {
            let path = String::from_utf8_lossy(path).to_string();
            (!path.is_empty()).then_some(path)
        })
        .collect()
}

fn exclude_deleted_paths(paths: Vec<String>, deleted_paths: Vec<String>) -> Vec<String> {
    let deleted_paths = deleted_paths.into_iter().collect::<BTreeSet<_>>();
    paths
        .into_iter()
        .filter(|path| !deleted_paths.contains(path))
        .collect()
}

#[cfg(test)]
#[path = "git_repo_tests.rs"]
mod tests;
