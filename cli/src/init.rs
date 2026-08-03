use crate::{
    agent_context::sync_repo_rules,
    api::{
        RepoInitResponse, api_url, create_repo, display_user, http_client, rollback_created_repo,
    },
    git_repo::{
        discover_git_repo, git_repo_has_head, install_scope_fetch_auth, warn_if_dirty_working_tree,
    },
    login::session_from_cache_or_browser,
    repo_config::{
        default_scope_repo_config, ensure_scope_repo_config_exists,
        mark_worktree_scope_repo_config_synced, repo_config_path,
    },
};
use anyhow::{Context, bail};
use std::{
    collections::BTreeSet,
    io::{self, Write},
    path::Path,
    process::{Command, Output},
};

pub fn run(name: Option<String>) -> anyhow::Result<()> {
    let git_repo = discover_git_repo("scope init")?;
    let has_head = git_repo_has_head(&git_repo);
    let api_url = api_url();
    let repo_name = match name.as_deref() {
        Some(name) => normalize_repo_name(name)?,
        None => prompt_repo_name(&git_repo.root)?,
    };
    let rules_sync = sync_repo_rules(&git_repo.root)?;
    if has_head {
        warn_if_dirty_working_tree(&git_repo)?;
    }

    let client = http_client()?;
    let session = session_from_cache_or_browser(&client, &api_url)?;
    eprintln!("Signed in as {}", display_user(&session.user));
    let created = create_repo(&client, &api_url, &session.token, repo_name)?;

    let remote_snapshot = match RemoteConfigSnapshot::capture(&git_repo.root, &created.init) {
        Ok(snapshot) => snapshot,
        Err(error) => {
            rollback_created_repo(&client, &api_url, &session.token, &created.repo);
            return Err(error);
        }
    };
    let config_created = match configure_remote(&git_repo.root, &created.init)
        .and_then(|_| ensure_scope_repo_config_exists(&git_repo.root))
        .and_then(|config_created| {
            mark_worktree_scope_repo_config_synced(&git_repo.root, &default_scope_repo_config())?;
            Ok(config_created)
        }) {
        Ok(config_created) => config_created,
        Err(error) => {
            let restore_result = remote_snapshot.restore(&git_repo.root);
            rollback_created_repo(&client, &api_url, &session.token, &created.repo);
            return match restore_result {
                Ok(()) => Err(error),
                Err(restore_error) => Err(error.context(format!(
                    "restore prior Git remote configuration: {restore_error:#}"
                ))),
            };
        }
    };

    println!(
        "Created Scope repo: {}/{}",
        created.repo.owner_handle, created.repo.name
    );
    println!("Configured Git remote: {}", created.init.remote_name);
    println!(
        "{} {}",
        if config_created {
            "Created"
        } else {
            "Using existing"
        },
        repo_config_path(&git_repo.root)?.display()
    );
    for path in &rules_sync.changed_paths {
        println!("Updated {}", path.display());
    }
    if !has_head {
        println!(
            "Create your first commit including the generated Scope files, then run: scope push"
        );
    } else if rules_sync.changed_paths.is_empty() {
        println!("Run: scope push");
    } else {
        println!("Commit the generated rules files, then run: scope push");
    }
    Ok(())
}

fn configure_remote(git_root: &Path, init: &RepoInitResponse) -> anyhow::Result<()> {
    let remotes = git_output(git_root, &["remote"])?;
    ensure_git_success(&remotes, "list Git remotes")?;
    let remote_exists = String::from_utf8_lossy(&remotes.stdout)
        .lines()
        .any(|remote| remote.trim() == init.remote_name);
    if remote_exists {
        for key in remote_section_keys(git_root, &init.remote_name)? {
            unset_local_config(git_root, &key)?;
        }
    }
    run_git_quiet(
        git_root,
        &["remote", "add", &init.remote_name, &init.git_remote_url],
        "add Scope Git remote",
    )?;
    install_scope_fetch_auth(git_root, &init.git_remote_url)
}

#[derive(Debug, Eq, PartialEq)]
struct RemoteConfigSnapshot {
    remote_name: String,
    values: Vec<(String, Vec<String>)>,
}

impl RemoteConfigSnapshot {
    fn capture(git_root: &Path, init: &RepoInitResponse) -> anyhow::Result<Self> {
        let keys = remote_config_keys(git_root, init)?;
        let values = keys
            .into_iter()
            .map(|key| {
                let values = local_config_values(git_root, &key)?;
                Ok((key, values))
            })
            .collect::<anyhow::Result<Vec<_>>>()?;
        Ok(Self {
            remote_name: init.remote_name.clone(),
            values,
        })
    }

    fn restore(self, git_root: &Path) -> anyhow::Result<()> {
        for key in remote_section_keys(git_root, &self.remote_name)? {
            unset_local_config(git_root, &key)?;
        }
        for (key, values) in self.values {
            replace_local_config_values(git_root, &key, &values)?;
        }
        Ok(())
    }
}

fn remote_config_keys(git_root: &Path, init: &RepoInitResponse) -> anyhow::Result<Vec<String>> {
    let mut keys = remote_section_keys(git_root, &init.remote_name)?;
    keys.extend([
        format!("credential.{}.helper", init.git_remote_url),
        format!("credential.{}.useHttpPath", init.git_remote_url),
    ]);
    Ok(keys)
}

fn remote_section_keys(git_root: &Path, remote_name: &str) -> anyhow::Result<Vec<String>> {
    let output = git_output(
        git_root,
        &[
            "config",
            "--local",
            "--name-only",
            "--null",
            "--get-regexp",
            "^remote\\.",
        ],
    )?;
    if !output.status.success() && output.status.code() != Some(1) {
        bail!(
            "list local Git remote config: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    let prefix = format!("remote.{remote_name}.");
    output
        .stdout
        .split(|byte| *byte == 0)
        .filter(|key| !key.is_empty())
        .map(|key| {
            String::from_utf8(key.to_vec()).context("local Git remote config key is not UTF-8")
        })
        .collect::<anyhow::Result<BTreeSet<_>>>()
        .map(|keys| {
            keys.into_iter()
                .filter(|key| key.starts_with(&prefix))
                .collect()
        })
}

fn local_config_values(git_root: &Path, key: &str) -> anyhow::Result<Vec<String>> {
    let output = git_output(git_root, &["config", "--local", "--null", "--get-all", key])?;
    if output.status.success() {
        let mut values = output.stdout.split(|byte| *byte == 0).collect::<Vec<_>>();
        if values.last().is_some_and(|value| value.is_empty()) {
            values.pop();
        }
        return values
            .into_iter()
            .map(|value| {
                String::from_utf8(value.to_vec())
                    .with_context(|| format!("local Git config {key} is not UTF-8"))
            })
            .collect();
    }
    if output.status.code() == Some(1) {
        return Ok(Vec::new());
    }
    bail!(
        "read local Git config {key}: {}",
        String::from_utf8_lossy(&output.stderr).trim()
    )
}

fn replace_local_config_values(
    git_root: &Path,
    key: &str,
    values: &[String],
) -> anyhow::Result<()> {
    unset_local_config(git_root, key)?;
    for value in values {
        run_git_quiet(
            git_root,
            &["config", "--local", "--add", key, value],
            &format!("restore local Git config {key}"),
        )?;
    }
    Ok(())
}

fn unset_local_config(git_root: &Path, key: &str) -> anyhow::Result<()> {
    let output = git_output(git_root, &["config", "--local", "--unset-all", key])?;
    if output.status.success() || output.status.code() == Some(5) {
        return Ok(());
    }
    bail!(
        "clear local Git config {key}: {}",
        String::from_utf8_lossy(&output.stderr).trim()
    )
}

fn run_git_quiet(git_root: &Path, args: &[&str], action: &str) -> anyhow::Result<()> {
    let output = git_output(git_root, args)?;
    ensure_git_success(&output, action)
}

fn git_output(git_root: &Path, args: &[&str]) -> anyhow::Result<Output> {
    Command::new("git")
        .current_dir(git_root)
        .args(args)
        .output()
        .with_context(|| format!("run git {}", args.join(" ")))
}

fn ensure_git_success(output: &Output, action: &str) -> anyhow::Result<()> {
    if output.status.success() {
        return Ok(());
    }
    let stderr = String::from_utf8_lossy(&output.stderr);
    let stderr = stderr.trim();
    if stderr.is_empty() {
        bail!("{action} failed");
    }
    bail!("{action} failed: {stderr}")
}

fn prompt_repo_name(git_root: &Path) -> anyhow::Result<String> {
    let default = default_repo_name(git_root);
    eprint!("Repository name [{default}]: ");
    io::stderr().flush().ok();

    let mut input = String::new();
    io::stdin()
        .read_line(&mut input)
        .context("read repository name")?;
    let name = if input.trim().is_empty() {
        default
    } else {
        input
    };
    normalize_repo_name(&name)
}

fn default_repo_name(git_root: &Path) -> String {
    git_root
        .file_name()
        .map(|name| name.to_string_lossy().to_string())
        .filter(|name| !name.trim().is_empty())
        .unwrap_or_else(|| "repo".to_string())
}

fn normalize_repo_name(name: &str) -> anyhow::Result<String> {
    let name = name.trim().to_ascii_lowercase();
    if name.is_empty() {
        bail!("repository name is required");
    }
    Ok(name)
}

#[cfg(test)]
#[path = "init_tests.rs"]
mod tests;
