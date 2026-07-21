use scope_core::{
    config::DEFAULT_GIT_BRANCH,
    db::GitCompactionCandidate,
    git_segments::GitStorageLimits,
    object_store::{ObjectStore, source_blob_bytes_bounded},
};
use scope_git_process::{ProcessLimits, run as run_process};
use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
    time::Duration,
};

pub(crate) fn build_compacted_pack(
    object_store: &dyn ObjectStore,
    candidate: &GitCompactionCandidate,
    storage_limits: GitStorageLimits,
    timeout: Duration,
) -> anyhow::Result<Vec<u8>> {
    let repo = TemporaryGitRepo::new()?;
    run_git(
        None,
        &["init", "--bare", repo.path.to_string_lossy().as_ref()],
        None,
        timeout,
        storage_limits.max_object_bytes(),
    )?;
    for segment in &candidate.segments {
        let bytes = source_blob_bytes_bounded(
            object_store,
            &segment.object,
            storage_limits.max_object_bytes(),
        )
        .map_err(|error| anyhow::anyhow!(error.message))?;
        run_git(
            Some(&repo.path),
            &["index-pack", "--stdin"],
            Some(bytes),
            timeout,
            storage_limits.max_object_bytes(),
        )?;
    }
    run_git(
        Some(&repo.path),
        &[
            "update-ref",
            &format!("refs/heads/{DEFAULT_GIT_BRANCH}"),
            &candidate.head.head_oid,
        ],
        None,
        timeout,
        storage_limits.max_object_bytes(),
    )?;
    run_git(
        Some(&repo.path),
        &["fsck", "--connectivity-only", &candidate.head.head_oid],
        None,
        timeout,
        storage_limits.max_object_bytes(),
    )?;
    run_git(
        Some(&repo.path),
        &["pack-objects", "--revs", "--stdout"],
        Some(format!("{}\n", candidate.head.head_oid).into_bytes()),
        timeout,
        storage_limits.max_object_bytes(),
    )
}

fn run_git(
    git_dir: Option<&Path>,
    args: &[&str],
    input: Option<Vec<u8>>,
    timeout: Duration,
    max_stdout_bytes: usize,
) -> anyhow::Result<Vec<u8>> {
    let mut command = Command::new("git");
    if let Some(git_dir) = git_dir {
        command.arg("--git-dir").arg(git_dir);
    }
    command.args(args);
    let output = run_process(
        &mut command,
        input,
        ProcessLimits::new(timeout).with_max_stdout_bytes(max_stdout_bytes),
        &format!("git {}", args.join(" ")),
    )
    .map_err(anyhow::Error::from)?;
    if !output.status.success() {
        anyhow::bail!(
            "git {} failed: {}",
            args.join(" "),
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    Ok(output.stdout)
}

struct TemporaryGitRepo {
    path: PathBuf,
}

impl TemporaryGitRepo {
    fn new() -> anyhow::Result<Self> {
        let mut random = [0_u8; 16];
        getrandom::fill(&mut random)
            .map_err(|error| anyhow::anyhow!("creating compaction path: {error}"))?;
        let path = std::env::temp_dir().join(format!(
            "scope-git-compact-{}-{}",
            std::process::id(),
            hex::encode(random)
        ));
        Ok(Self { path })
    }
}

impl Drop for TemporaryGitRepo {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use scope_git_process::ProcessError;

    #[test]
    fn worker_git_output_obeys_exact_byte_limit() {
        let exact = run_git(
            None,
            &["hash-object", "--stdin"],
            Some(b"content".to_vec()),
            Duration::from_secs(1),
            41,
        )
        .unwrap();
        assert_eq!(exact.len(), 41);

        let error = run_git(
            None,
            &["hash-object", "--stdin"],
            Some(b"content".to_vec()),
            Duration::from_secs(1),
            40,
        )
        .unwrap_err();
        assert!(
            error
                .downcast_ref::<ProcessError>()
                .is_some_and(ProcessError::is_stdout_limit)
        );
    }
}
