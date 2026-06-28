use anyhow::{Context, bail};
use std::{
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

pub fn ensure_git_repo_ready(command_name: &str) -> anyhow::Result<GitRepo> {
    let root_output = Command::new("git")
        .args(["rev-parse", "--show-toplevel"])
        .output()
        .context("inspect Git repository")?;
    if !root_output.status.success() {
        bail!("run {command_name} from inside an existing Git repository");
    }

    if !git_success(&["rev-parse", "--verify", "HEAD"]) {
        bail!("create at least one Git commit before running {command_name}");
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

pub fn warn_if_dirty_working_tree(repo: &GitRepo) -> anyhow::Result<()> {
    let output = Command::new("git")
        .current_dir(&repo.root)
        .args(["status", "--porcelain"])
        .output()
        .context("inspect Git working tree")?;
    if !output.status.success() {
        bail!("git status --porcelain failed");
    }
    if !output.stdout.is_empty() {
        eprintln!("Working tree has uncommitted changes.");
        eprintln!("Only committed HEAD will be pushed to Scope.");
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
    branch: &str,
    bearer_token: &str,
) -> anyhow::Result<()> {
    let inherited_config_count = env::var("GIT_CONFIG_COUNT")
        .ok()
        .and_then(|value| value.parse::<usize>().ok());
    let plan = git_push_auth_plan(destination, branch, bearer_token, inherited_config_count);
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

pub fn git_push_auth_plan(
    destination: &str,
    branch: &str,
    bearer_token: &str,
    inherited_config_count: Option<usize>,
) -> GitPushAuthPlan {
    let config_index = inherited_config_count.unwrap_or(0);
    GitPushAuthPlan {
        args: vec![
            "-c".to_string(),
            "push.recurseSubmodules=no".to_string(),
            "push".to_string(),
            destination.to_string(),
            format!("HEAD:{branch}"),
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

fn git_output(args: &[&str]) -> anyhow::Result<Output> {
    Command::new("git")
        .args(args)
        .output()
        .with_context(|| format!("run git {}", args.join(" ")))
}

fn git_success(args: &[&str]) -> bool {
    Command::new("git")
        .args(args)
        .status()
        .is_ok_and(|status| status.success())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn git_push_auth_plan_keeps_bearer_token_out_of_process_args() {
        let plan = git_push_auth_plan(
            "https://scope.example/git/adam/random",
            "main",
            "scope_cli_secret",
            Some(2),
        );

        assert_eq!(
            plan.args,
            vec![
                "-c",
                "push.recurseSubmodules=no",
                "push",
                "https://scope.example/git/adam/random",
                "HEAD:main"
            ]
        );
        assert!(!plan.args.iter().any(|arg| arg.contains("scope_cli_secret")));
        assert_eq!(
            plan.env,
            vec![
                ("GIT_CONFIG_COUNT".to_string(), "3".to_string()),
                (
                    "GIT_CONFIG_KEY_2".to_string(),
                    "http.https://scope.example/git/adam/random.extraHeader".to_string()
                ),
                (
                    "GIT_CONFIG_VALUE_2".to_string(),
                    "Authorization: Bearer scope_cli_secret".to_string()
                ),
            ]
        );
    }
}
