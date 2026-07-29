use super::{command_stdout, command_success};
use anyhow::{Context, bail};
use std::{
    path::Path,
    process::{Command, Stdio},
};

const MAX_CHECKOUT_FILES: u64 = 100_000;
const MAX_CHECKOUT_BYTES: u64 = 10 * 1024 * 1024 * 1024;

pub(super) fn checkout_exact_commit(
    bundle_path: &Path,
    workspace: &Path,
    git_oid: &str,
) -> anyhow::Result<()> {
    command_success(
        Command::new("git")
            .args(["clone", "--no-local", "--no-checkout"])
            .arg(bundle_path)
            .arg(workspace),
        "clone run source bundle without checking out files",
    )?;
    let output = Command::new("git")
        .current_dir(workspace)
        .args(["ls-tree", "-r", "-z", "--long", git_oid])
        .stdin(Stdio::null())
        .output()
        .context("inspect exact run commit tree")?;
    if !output.status.success() {
        bail!(
            "inspect exact run commit tree: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    validate_checkout_tree(&output.stdout)?;
    command_success(
        Command::new("git")
            .current_dir(workspace)
            .args(["checkout", "--detach", git_oid]),
        "check out exact run commit",
    )?;
    let actual_oid = command_stdout(
        Command::new("git")
            .current_dir(workspace)
            .args(["rev-parse", "HEAD"]),
        "verify checked-out run commit",
    )?;
    if actual_oid.trim() != git_oid {
        bail!("checked-out commit does not match the claimed job");
    }
    Ok(())
}

fn validate_checkout_tree(tree: &[u8]) -> anyhow::Result<()> {
    let mut files = 0_u64;
    let mut bytes = 0_u64;
    for record in tree.split(|byte| *byte == 0).filter(|record| !record.is_empty()) {
        let separator = record
            .iter()
            .position(|byte| *byte == b'\t')
            .context("Git tree entry is missing its path")?;
        let metadata =
            std::str::from_utf8(&record[..separator]).context("Git tree metadata is not UTF-8")?;
        let mut fields = metadata.split_ascii_whitespace();
        let _mode = fields.next().context("Git tree entry is missing its mode")?;
        let kind = fields.next().context("Git tree entry is missing its type")?;
        let _oid = fields.next().context("Git tree entry is missing its object id")?;
        let size = fields.next().context("Git tree entry is missing its size")?;
        if fields.next().is_some() || kind != "blob" {
            bail!("run checkout contains an unsupported Git tree entry");
        }
        let size = size
            .parse::<u64>()
            .context("Git tree entry has an invalid blob size")?;
        files = files.checked_add(1).context("run checkout file count overflow")?;
        bytes = bytes
            .checked_add(size)
            .context("run checkout byte count overflow")?;
        if files > MAX_CHECKOUT_FILES {
            bail!("run checkout exceeds the {MAX_CHECKOUT_FILES} file limit");
        }
        if bytes > MAX_CHECKOUT_BYTES {
            bail!("run checkout exceeds the {MAX_CHECKOUT_BYTES} byte limit");
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn checkout_tree_budget_counts_files_and_expanded_blob_bytes() {
        validate_checkout_tree(
            b"100644 blob aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa 12\tone.txt\0\
              120000 blob bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb 5\tlink\0",
        )
        .unwrap();

        let oversized = format!(
            "100644 blob {} {}\thuge.bin\0",
            "a".repeat(40),
            MAX_CHECKOUT_BYTES + 1
        );
        assert!(
            validate_checkout_tree(oversized.as_bytes())
                .unwrap_err()
                .to_string()
                .contains("byte limit")
        );
    }

    #[test]
    fn checkout_tree_budget_rejects_gitlinks() {
        let tree = format!(
            "160000 commit {} -\tthird-party\0",
            "a".repeat(40)
        );
        assert!(
            validate_checkout_tree(tree.as_bytes())
                .unwrap_err()
                .to_string()
                .contains("unsupported")
        );
    }
}
