use anyhow::{Context, bail};
use reqwest::Url;
use std::{
    collections::BTreeSet,
    env,
    path::{Path, PathBuf},
    process::{Command, Output},
};

#[derive(Debug)]
pub struct GitRepo {
    pub root: PathBuf,
}

#[derive(Debug, Eq, PartialEq)]
pub struct GitCommandPlan {
    pub args: Vec<String>,
    pub env: Vec<(String, String)>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GitChangedPath {
    pub status: String,
    pub path: String,
}

const SCOPE_GIT_CREDENTIAL_HELPER: &str = "!scope git-credential";
const SCOPE_API_URL_CONFIG_KEY: &str = "scope.apiUrl";
const SCOPE_GIT_ORIGIN_CONFIG_KEY: &str = "scope.gitOrigin";

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
    if !git_repo_has_head(&repo) {
        bail!("create at least one Git commit before running {command_name}");
    }

    Ok(repo)
}

pub fn git_repo_has_head(repo: &GitRepo) -> bool {
    git_success_in_repo(repo, &["rev-parse", "--verify", "HEAD"])
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
    if has_dirty_paths(&output.stdout) {
        eprintln!("Working tree has uncommitted changes.");
        eprintln!("Only committed HEAD will be pushed to Scope.");
    }
    Ok(())
}

pub fn ensure_clean_working_tree(repo: &GitRepo, command_name: &str) -> anyhow::Result<()> {
    let output = Command::new("git")
        .current_dir(&repo.root)
        .args(["status", "--porcelain", "--untracked-files=all"])
        .output()
        .context("inspect Git working tree")?;
    if !output.status.success() {
        bail!("git status --porcelain failed");
    }
    if has_dirty_paths(&output.stdout) {
        bail!("commit or stash local changes before running {command_name}");
    }
    Ok(())
}

fn has_dirty_paths(status: &[u8]) -> bool {
    String::from_utf8_lossy(status)
        .lines()
        .any(|line| !line.trim().is_empty())
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

pub fn request_side_changed_file_paths(
    repo: &GitRepo,
    recorded_base_oid: &str,
    current_main_oid: &str,
    request_head_oid: &str,
) -> anyhow::Result<Vec<String>> {
    ensure_commit_exists(repo, recorded_base_oid, "recorded request base")?;
    ensure_commit_exists(repo, current_main_oid, "current main")?;
    ensure_commit_exists(repo, request_head_oid, "request head")?;

    let merge_base_output = git_output_in_repo(
        repo,
        &["merge-base", "--all", current_main_oid, request_head_oid],
    )?;
    if !merge_base_output.status.success() {
        if merge_base_output.status.code() == Some(1) {
            bail!(
                "current main and request head have unrelated Git histories; Scope requests must descend from the repository's Scope main. Run `scope request start <name>` and replay the GitHub branch changes onto that request branch before `scope push`"
            );
        }
        bail!("find the request branch merge base failed");
    }
    let merge_base_oids = String::from_utf8_lossy(&merge_base_output.stdout)
        .lines()
        .map(str::trim)
        .filter(|oid| !oid.is_empty())
        .map(ToString::to_string)
        .collect::<Vec<_>>();
    let merge_base_oid = match merge_base_oids.as_slice() {
        [] => bail!("Git did not return a request branch merge base"),
        [merge_base_oid] => merge_base_oid,
        _ => bail!("current main and request head have multiple Git merge bases"),
    };
    ensure_recorded_base_ancestor_of_merge_base(repo, recorded_base_oid, merge_base_oid)?;

    let request_output = git_output_in_repo(
        repo,
        &[
            "diff",
            "--name-only",
            "-z",
            "--no-renames",
            merge_base_oid,
            request_head_oid,
        ],
    )?;
    if !request_output.status.success() {
        bail!("inspect request-side committed paths failed");
    }

    let merge_output = git_output_in_repo(
        repo,
        &[
            "merge-tree",
            "--write-tree",
            "--no-messages",
            "--name-only",
            "-z",
            current_main_oid,
            request_head_oid,
        ],
    )?;
    if !merge_output.status.success() && merge_output.status.code() != Some(1) {
        bail!("compute the request merge result failed");
    }
    let merge_tree_separator = merge_output
        .stdout
        .iter()
        .position(|byte| *byte == 0)
        .ok_or_else(|| anyhow::anyhow!("Git did not return a request merge result tree"))?;
    let merge_tree_oid =
        String::from_utf8_lossy(&merge_output.stdout[..merge_tree_separator]).to_string();
    if merge_tree_oid.is_empty() {
        bail!("Git did not return a request merge result tree");
    }
    let conflict_paths = parse_nul_paths(&merge_output.stdout[merge_tree_separator + 1..]);

    let merge_result_output = git_output_in_repo(
        repo,
        &[
            "diff",
            "--name-only",
            "-z",
            "--no-renames",
            current_main_oid,
            &merge_tree_oid,
        ],
    )?;
    if !merge_result_output.status.success() {
        bail!("inspect request merge result paths failed");
    }
    let mut paths = parse_nul_paths(&request_output.stdout)
        .into_iter()
        .collect::<BTreeSet<_>>();
    paths.extend(parse_nul_paths(&merge_result_output.stdout));
    paths.extend(conflict_paths);
    Ok(paths.into_iter().collect())
}

fn ensure_recorded_base_ancestor_of_merge_base(
    repo: &GitRepo,
    recorded_base_oid: &str,
    merge_base_oid: &str,
) -> anyhow::Result<()> {
    let output = git_output_in_repo(
        repo,
        &[
            "merge-base",
            "--is-ancestor",
            recorded_base_oid,
            merge_base_oid,
        ],
    )?;
    if output.status.success() {
        return Ok(());
    }
    if output.status.code() == Some(1) {
        bail!("request branch merge base does not descend from the recorded request base");
    }
    bail!("validate request branch ancestry failed")
}

fn ensure_commit_exists(repo: &GitRepo, revision: &str, label: &str) -> anyhow::Result<()> {
    let commit = format!("{revision}^{{commit}}");
    let output = git_output_in_repo(repo, &["rev-parse", "--verify", "--quiet", &commit])?;
    if !output.status.success() {
        bail!("{label} commit is missing from the local Git repository");
    }
    Ok(())
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

pub fn git_remote_push_url(repo: &GitRepo, remote: &str) -> anyhow::Result<String> {
    let output = git_output_in_repo(repo, &["remote", "get-url", "--push", remote])?;
    if !output.status.success() {
        bail!("Scope remote '{remote}' is not configured. Run: scope init");
    }

    let url = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if url.is_empty() {
        bail!("Scope remote '{remote}' has an empty push URL");
    }
    Ok(url)
}

pub fn git_remote_fetch_url(repo: &GitRepo, remote: &str) -> anyhow::Result<String> {
    let output = git_output_in_repo(repo, &["remote", "get-url", remote])?;
    if !output.status.success() {
        bail!("Scope remote '{remote}' is not configured. Run: scope init");
    }

    let url = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if url.is_empty() {
        bail!("Scope remote '{remote}' has an empty fetch URL");
    }
    Ok(url)
}

pub fn git_remote_names(repo: &GitRepo) -> anyhow::Result<Vec<String>> {
    let output = git_output_in_repo(repo, &["remote"])?;
    if !output.status.success() {
        bail!("list Git remotes failed");
    }

    Ok(String::from_utf8_lossy(&output.stdout)
        .lines()
        .map(str::trim)
        .filter(|remote| !remote.is_empty())
        .map(str::to_string)
        .collect())
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

pub fn run_git_in_repo(repo: &GitRepo, args: &[&str]) -> anyhow::Result<()> {
    let status = Command::new("git")
        .current_dir(&repo.root)
        .args(args)
        .status()
        .with_context(|| format!("run git {}", args.join(" ")))?;
    if !status.success() {
        bail!("git {} failed", args.join(" "));
    }
    Ok(())
}

pub fn try_run_git_in_repo(repo: &GitRepo, args: &[&str]) -> anyhow::Result<bool> {
    let status = Command::new("git")
        .current_dir(&repo.root)
        .args(args)
        .status()
        .with_context(|| format!("run git {}", args.join(" ")))?;
    Ok(status.success())
}

pub fn git_text_in_repo(repo: &GitRepo, args: &[&str]) -> anyhow::Result<String> {
    let output = git_output_in_repo(repo, args)?;
    if !output.status.success() {
        bail!("git {} failed", args.join(" "));
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

pub fn current_branch(repo: &GitRepo) -> anyhow::Result<String> {
    let branch = git_text_in_repo(repo, &["branch", "--show-current"])?;
    if branch.is_empty() {
        bail!("request commands require a named local branch");
    }
    Ok(branch)
}

pub fn branch_config_value(
    repo: &GitRepo,
    branch: &str,
    key: &str,
) -> anyhow::Result<Option<String>> {
    let config_key = format!("branch.{branch}.{key}");
    let output = git_output_in_repo(repo, &["config", "--get", &config_key])?;
    if output.status.success() {
        let value = String::from_utf8_lossy(&output.stdout).trim().to_string();
        return Ok((!value.is_empty()).then_some(value));
    }
    Ok(None)
}

pub fn set_branch_config_value(
    repo: &GitRepo,
    branch: &str,
    key: &str,
    value: &str,
) -> anyhow::Result<()> {
    let config_key = format!("branch.{branch}.{key}");
    run_git_in_repo(repo, &["config", "--local", &config_key, value])
}

pub fn push_head_with_bearer(
    destination: &str,
    commit_oid: &str,
    branch: &str,
    bearer_token: &str,
    push_intent_token: &str,
) -> anyhow::Result<()> {
    let plan = git_push_auth_plan(
        destination,
        commit_oid,
        branch,
        bearer_token,
        push_intent_token,
        inherited_git_config_count(),
    );
    run_git_plan_status(
        plan,
        None,
        "run authenticated Scope git push",
        "git push to Scope failed",
    )
}

pub fn push_head_to_ref_with_bearer(
    destination: &str,
    commit_oid: &str,
    refname: &str,
    bearer_token: &str,
) -> anyhow::Result<()> {
    let plan = git_auth_plan(
        vec![
            "-c".to_string(),
            "push.recurseSubmodules=no".to_string(),
            "push".to_string(),
            destination.to_string(),
            format!("{commit_oid}:{refname}"),
        ],
        destination,
        &[format!("Authorization: Bearer {bearer_token}")],
        inherited_git_config_count(),
    );
    run_git_plan_output(
        plan,
        None,
        "run authenticated Scope request branch push",
        "git push to Scope request ref failed",
    )
}

pub fn clone_with_bearer(
    remote_url: &str,
    bearer_token: &str,
    destination: Option<&Path>,
) -> anyhow::Result<()> {
    let plan = git_clone_auth_plan(
        remote_url,
        bearer_token,
        destination,
        inherited_git_config_count(),
    );
    run_git_plan_status(
        plan,
        None,
        "run authenticated Scope git clone",
        "git clone from Scope failed",
    )
}

pub fn install_scope_fetch_auth(
    repo_root: &Path,
    remote_url: &str,
    api_url: &str,
) -> anyhow::Result<()> {
    let helper_key = credential_config_key(remote_url, "helper")?;
    let use_http_path_key = credential_config_key(remote_url, "useHttpPath")?;
    let git_origin = transport_origin(remote_url)?;
    let unset = Command::new("git")
        .current_dir(repo_root)
        .args(["config", "--local", "--unset-all", &helper_key])
        .status()
        .context("clear existing Scope Git credential helpers")?;
    if !unset.success() && unset.code() != Some(5) {
        bail!("clear existing Scope Git credential helpers failed");
    }
    run_git_config(repo_root, &["--add", &helper_key, ""])?;
    run_git_config(
        repo_root,
        &["--add", &helper_key, SCOPE_GIT_CREDENTIAL_HELPER],
    )?;
    run_git_config(repo_root, &["--replace-all", &use_http_path_key, "true"])?;
    run_git_config(
        repo_root,
        &["--replace-all", SCOPE_API_URL_CONFIG_KEY, api_url],
    )?;
    run_git_config(
        repo_root,
        &["--replace-all", SCOPE_GIT_ORIGIN_CONFIG_KEY, &git_origin],
    )?;
    Ok(())
}

pub fn scope_git_origin(repo: &GitRepo, fallback_url: &str) -> anyhow::Result<String> {
    let output = git_output_in_repo(
        repo,
        &["config", "--local", "--get", SCOPE_GIT_ORIGIN_CONFIG_KEY],
    )?;
    if output.status.success() {
        let origin = String::from_utf8_lossy(&output.stdout).trim().to_string();
        if !origin.is_empty() {
            return Ok(origin);
        }
    } else if output.status.code() != Some(1) {
        bail!("read configured Scope Git origin failed");
    }
    transport_origin(fallback_url)
}

pub fn scope_api_url_from_git_config(repo_root: &Path) -> anyhow::Result<Option<String>> {
    let output = Command::new("git")
        .current_dir(repo_root)
        .args(["config", "--local", "--get", SCOPE_API_URL_CONFIG_KEY])
        .output()
        .context("read configured Scope API URL")?;
    if output.status.success() {
        let api_url = String::from_utf8_lossy(&output.stdout).trim().to_string();
        return Ok((!api_url.is_empty()).then_some(api_url));
    }
    if output.status.code() == Some(1) {
        return Ok(None);
    }
    bail!("read configured Scope API URL failed")
}

fn transport_origin(value: &str) -> anyhow::Result<String> {
    let mut url = Url::parse(value).context("parse Scope transport URL")?;
    let _ = url.set_username("");
    let _ = url.set_password(None);
    url.set_path("");
    url.set_query(None);
    url.set_fragment(None);
    Ok(url.as_str().trim_end_matches('/').to_string())
}

fn run_git_config(repo_root: &Path, args: &[&str]) -> anyhow::Result<()> {
    let status = Command::new("git")
        .current_dir(repo_root)
        .args(["config", "--local"])
        .args(args)
        .status()
        .context("configure Scope Git credential helper")?;
    if !status.success() {
        bail!("configure Scope Git credential helper failed");
    }
    Ok(())
}

fn credential_config_key(remote_url: &str, name: &str) -> anyhow::Result<String> {
    if remote_url.chars().any(char::is_control) {
        bail!("Scope Git remote URL cannot contain control characters");
    }
    Ok(format!("credential.{remote_url}.{name}"))
}

pub fn fetch_scope_remote_with_bearer(
    repo: &GitRepo,
    destination: &str,
    remote: &str,
    branch: &str,
    bearer_token: &str,
) -> anyhow::Result<()> {
    let plan = git_fetch_auth_plan(
        destination,
        remote,
        branch,
        bearer_token,
        inherited_git_config_count(),
    );
    run_git_plan_output(
        plan,
        Some(&repo.root),
        "refresh Scope Git remote before push review",
        "refresh Scope Git remote before push review failed",
    )
}

pub fn git_clone_auth_plan(
    remote_url: &str,
    bearer_token: &str,
    destination: Option<&Path>,
    inherited_config_count: Option<usize>,
) -> GitCommandPlan {
    let mut args = vec!["clone".to_string(), remote_url.to_string()];
    if let Some(destination) = destination {
        args.push(destination.to_string_lossy().to_string());
    }
    git_auth_plan(
        args,
        remote_url,
        &[format!("Authorization: Bearer {bearer_token}")],
        inherited_config_count,
    )
}

pub fn git_push_auth_plan(
    destination: &str,
    commit_oid: &str,
    branch: &str,
    bearer_token: &str,
    push_intent_token: &str,
    inherited_config_count: Option<usize>,
) -> GitCommandPlan {
    git_auth_plan(
        vec![
            "-c".to_string(),
            "push.recurseSubmodules=no".to_string(),
            "push".to_string(),
            destination.to_string(),
            format!("{commit_oid}:refs/heads/{branch}"),
        ],
        destination,
        &[
            format!("Authorization: Bearer {bearer_token}"),
            format!("X-Scope-Push-Intent: {push_intent_token}"),
        ],
        inherited_config_count,
    )
}

pub fn git_fetch_auth_plan(
    destination: &str,
    remote: &str,
    branch: &str,
    bearer_token: &str,
    inherited_config_count: Option<usize>,
) -> GitCommandPlan {
    git_auth_plan(
        vec![
            "-c".to_string(),
            "protocol.version=2".to_string(),
            "fetch".to_string(),
            "--no-tags".to_string(),
            destination.to_string(),
            format!("+refs/heads/{branch}:refs/remotes/{remote}/{branch}"),
        ],
        destination,
        &[format!("Authorization: Bearer {bearer_token}")],
        inherited_config_count,
    )
}

fn git_auth_plan(
    args: Vec<String>,
    destination: &str,
    headers: &[String],
    inherited_config_count: Option<usize>,
) -> GitCommandPlan {
    let first_index = inherited_config_count.unwrap_or(0);
    let mut env = vec![(
        "GIT_CONFIG_COUNT".to_string(),
        (first_index + headers.len()).to_string(),
    )];
    for (offset, header) in headers.iter().enumerate() {
        let index = first_index + offset;
        env.push((
            format!("GIT_CONFIG_KEY_{index}"),
            format!("http.{destination}.extraHeader"),
        ));
        env.push((format!("GIT_CONFIG_VALUE_{index}"), header.clone()));
    }
    GitCommandPlan { args, env }
}

fn inherited_git_config_count() -> Option<usize> {
    env::var("GIT_CONFIG_COUNT")
        .ok()
        .and_then(|value| value.parse().ok())
}

fn git_command(plan: GitCommandPlan, cwd: Option<&Path>) -> Command {
    let mut command = Command::new("git");
    if let Some(cwd) = cwd {
        command.current_dir(cwd);
    }
    command.args(plan.args);
    command.envs(plan.env);
    command
}

fn run_git_plan_status(
    plan: GitCommandPlan,
    cwd: Option<&Path>,
    context: &str,
    failure: &str,
) -> anyhow::Result<()> {
    if !git_command(plan, cwd)
        .status()
        .with_context(|| context.to_string())?
        .success()
    {
        bail!("{failure}");
    }
    Ok(())
}

fn run_git_plan_output(
    plan: GitCommandPlan,
    cwd: Option<&Path>,
    context: &str,
    failure: &str,
) -> anyhow::Result<()> {
    let output = git_command(plan, cwd)
        .output()
        .with_context(|| context.to_string())?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let stderr = stderr.trim();
        if !stderr.is_empty() {
            bail!("{failure}: {stderr}");
        }
        bail!("{failure}");
    }
    Ok(())
}

pub fn head_oid(repo: &GitRepo) -> anyhow::Result<String> {
    let output = git_output_in_repo(repo, &["rev-parse", "HEAD"])?;
    if !output.status.success() {
        bail!("inspect Git HEAD failed");
    }

    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
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
        .output()
        .is_ok_and(|output| output.status.success())
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
