use super::{ClaimRunResponse, RunnerConfig, runner_client, runner_work_root};
use crate::api::abandon_attempt;
use anyhow::{Context, bail};
use std::{fs, io::Write, path::Path, process::Command};

const RECOVERY_CLAIM_FILE: &str = "claim.json";

pub(super) fn persist_recovery_claim(
    work_dir: &Path,
    claim: &ClaimRunResponse,
) -> anyhow::Result<()> {
    let path = work_dir.join(RECOVERY_CLAIM_FILE);
    #[cfg(unix)]
    let mut file = {
        use std::os::unix::fs::OpenOptionsExt;
        fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .mode(0o600)
            .open(&path)
            .context("create runner recovery claim")?
    };
    #[cfg(not(unix))]
    let mut file = fs::OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(&path)
        .context("create runner recovery claim")?;
    serde_json::to_writer(&mut file, claim).context("serialize runner recovery claim")?;
    file.write_all(b"\n")?;
    file.sync_all().context("persist runner recovery claim")
}

pub(super) fn reconcile_runner_state(config: &RunnerConfig) -> anyhow::Result<()> {
    reconcile_runner_containers(&config.runner_id)?;
    let root = runner_work_root()?;
    if !root.exists() {
        return Ok(());
    }
    validate_work_root(&root)?;
    let client = runner_client()?;
    for entry in fs::read_dir(&root).context("read runner work root")? {
        let entry = entry.context("read runner work entry")?;
        let file_type = entry.file_type().context("inspect runner work entry")?;
        if !file_type.is_dir() || file_type.is_symlink() {
            continue;
        }
        let claim_path = entry.path().join(RECOVERY_CLAIM_FILE);
        if !claim_path.is_file() {
            continue;
        }
        let claim: ClaimRunResponse = serde_json::from_slice(
            &fs::read(&claim_path)
                .with_context(|| format!("read recovery claim {}", claim_path.display()))?,
        )
        .with_context(|| format!("parse recovery claim {}", claim_path.display()))?;
        abandon_attempt(
            &client,
            &config.api_url,
            &claim.attempt_token,
            &claim.attempt_id,
        )?;
    }
    cleanup_work_root(&root)
}

fn reconcile_runner_containers(runner_id: &str) -> anyhow::Result<()> {
    let output = Command::new("docker")
        .args([
            "ps",
            "-aq",
            "--filter",
            &format!("label=scope.runner-id={runner_id}"),
        ])
        .output()
        .context("list stale Scope runner containers")?;
    if !output.status.success() {
        bail!(
            "list stale Scope runner containers: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    for container_id in String::from_utf8_lossy(&output.stdout)
        .lines()
        .map(str::trim)
        .filter(|id| !id.is_empty())
    {
        if !super::supervisor::terminate_container(container_id) {
            bail!("could not confirm stale Scope container {container_id} was removed");
        }
    }
    Ok(())
}

fn validate_work_root(root: &Path) -> anyhow::Result<()> {
    let metadata = fs::symlink_metadata(root).context("inspect runner work root")?;
    if !metadata.file_type().is_dir() || metadata.file_type().is_symlink() {
        bail!(
            "runner work root is not a real directory: {}",
            root.display()
        );
    }
    Ok(())
}

pub(super) fn cleanup_work_root(root: &Path) -> anyhow::Result<()> {
    for entry in fs::read_dir(root).context("read runner work root")? {
        let entry = entry.context("read runner work entry")?;
        let file_type = entry.file_type().context("inspect runner work entry")?;
        let path = entry.path();
        if file_type.is_dir() && !file_type.is_symlink() {
            fs::remove_dir_all(&path)
                .with_context(|| format!("remove stale runner work {}", path.display()))?;
        } else {
            fs::remove_file(&path)
                .with_context(|| format!("remove stale runner work {}", path.display()))?;
        }
    }
    Ok(())
}
