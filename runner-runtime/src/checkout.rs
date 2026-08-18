use anyhow::{Context as _, bail};
use std::{
    path::Path,
    process::{Command, Stdio},
};

const MAX_CHECKOUT_FILES: u64 = 100_000;
const MAX_CHECKOUT_BYTES: u64 = 10 * 1024 * 1024 * 1024;

pub fn checkout_exact_commit(bundle: &Path, workspace: &Path, git_oid: &str) -> anyhow::Result<()> {
    command(
        Command::new("git")
            .args(["clone", "--no-local", "--no-checkout"])
            .arg(bundle)
            .arg(workspace),
        "clone source bundle",
    )?;
    let output = Command::new("git")
        .current_dir(workspace)
        .args(["ls-tree", "-r", "-z", "--long", git_oid])
        .stdin(Stdio::null())
        .output()
        .context("inspect checkout tree")?;
    if !output.status.success() {
        bail!(
            "inspect checkout tree: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    validate_tree(&output.stdout)?;
    command(
        Command::new("git")
            .current_dir(workspace)
            .args(["checkout", "--detach", git_oid]),
        "checkout exact commit",
    )?;
    let output = Command::new("git")
        .current_dir(workspace)
        .args(["rev-parse", "HEAD"])
        .output()
        .context("verify checkout")?;
    if !output.status.success() || String::from_utf8_lossy(&output.stdout).trim() != git_oid {
        bail!("checked-out commit does not match attempt");
    }
    Ok(())
}

fn command(command: &mut Command, label: &str) -> anyhow::Result<()> {
    let output = command
        .stdin(Stdio::null())
        .output()
        .with_context(|| label.to_string())?;
    if !output.status.success() {
        bail!(
            "{label}: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    Ok(())
}

fn validate_tree(tree: &[u8]) -> anyhow::Result<()> {
    let mut files = 0_u64;
    let mut bytes = 0_u64;
    for record in tree
        .split(|byte| *byte == 0)
        .filter(|record| !record.is_empty())
    {
        let split = record
            .iter()
            .position(|byte| *byte == b'\t')
            .context("tree entry path missing")?;
        let mut fields = std::str::from_utf8(&record[..split])?.split_ascii_whitespace();
        let _mode = fields.next().context("tree mode missing")?;
        if fields.next() != Some("blob") {
            bail!("checkout contains unsupported Git entry");
        }
        let _oid = fields.next().context("tree object missing")?;
        let size = fields.next().context("tree size missing")?.parse::<u64>()?;
        if fields.next().is_some() {
            bail!("tree metadata is invalid");
        }
        files += 1;
        bytes = bytes.checked_add(size).context("checkout size overflow")?;
        if files > MAX_CHECKOUT_FILES {
            bail!("checkout exceeds file limit");
        }
        if bytes > MAX_CHECKOUT_BYTES {
            bail!("checkout exceeds byte limit");
        }
    }
    Ok(())
}
