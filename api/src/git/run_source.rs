use crate::{
    error::ApiError,
    git::{storage::restore_git_pack_spans, upload::git_process_output_with_limits},
    persistence::ensure_private_dir,
    state::AppState,
};
use scope_domain::runs::run::RunSource;
use scope_git::DEFAULT_GIT_BRANCH;
use scope_object_store::source_blob_bytes_bounded;
use sha2::{Digest as _, Sha256};
use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
    sync::atomic::{AtomicU64, Ordering},
};

static RUN_SOURCE_ATTEMPT: AtomicU64 = AtomicU64::new(1);

pub(crate) struct MaterializedRunSource {
    pub(crate) bytes: Vec<u8>,
    pub(crate) sha256: String,
}

pub(crate) fn materialize_run_source_bundle(
    state: &AppState,
    source: &RunSource,
    max_bytes: usize,
) -> Result<MaterializedRunSource, ApiError> {
    let bytes = if let Some(bundle) = source.ephemeral_bundle() {
        source_blob_bytes_bounded(state.object_store.as_ref(), bundle, max_bytes)?
    } else {
        materialize_accepted_git_head_bundle(state, source, max_bytes)?
    };
    Ok(MaterializedRunSource {
        sha256: hex::encode(Sha256::digest(&bytes)),
        bytes,
    })
}

fn materialize_accepted_git_head_bundle(
    state: &AppState,
    source: &RunSource,
    max_bytes: usize,
) -> Result<Vec<u8>, ApiError> {
    let (_, head, pack_spans) = source.logical_git_head().ok_or_else(|| {
        ApiError::internal_message("run source does not contain a materializable Git head")
    })?;
    let repo = TemporaryRunSourceRepository::new(state)?;
    restore_git_pack_spans(state, head, pack_spans, repo.path())?;
    let main_ref = format!("refs/heads/{DEFAULT_GIT_BRANCH}");
    let output = git_process_output_with_limits(
        Command::new("git")
            .arg("--git-dir")
            .arg(repo.path())
            .args(["bundle", "create", "-", &main_ref]),
        None,
        state.runtime_budgets.git_command_timeout(),
        max_bytes,
    )?;
    if !output.status.success() {
        return Err(ApiError::infrastructure_unavailable(format!(
            "materializing run source bundle: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        )));
    }
    Ok(output.stdout)
}

struct TemporaryRunSourceRepository {
    path: PathBuf,
}

impl TemporaryRunSourceRepository {
    fn new(state: &AppState) -> Result<Self, ApiError> {
        let root = state.data_dir.join("run-source");
        ensure_private_dir(&root)?;
        let attempt = RUN_SOURCE_ATTEMPT.fetch_add(1, Ordering::Relaxed);
        let path = root.join(format!("{}.{}.git", std::process::id(), attempt));
        if path.exists() {
            return Err(ApiError::internal_message(
                "run source temporary repository path already exists",
            ));
        }
        Ok(Self { path })
    }

    fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for TemporaryRunSourceRepository {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::git::import::{git_push_from_repo, run_git};
    use scope_domain::{projection::ProjectionViewKey, runs::run::RunSource, store::GitHead};

    #[tokio::test]
    async fn accepted_git_head_materializes_a_non_durable_bundle_at_run_start() {
        let state = AppState::test_state();
        let repository = TemporaryRunSourceRepository::new(&state).unwrap();
        fs::create_dir_all(repository.path()).unwrap();
        run_git(
            None,
            &["init", "-b", "main", repository.path().to_str().unwrap()],
            "initialize source repo",
        )
        .unwrap();
        run_git(
            Some(repository.path()),
            &["config", "user.email", "scope@test.invalid"],
            "configure source repository email",
        )
        .unwrap();
        run_git(
            Some(repository.path()),
            &["config", "user.name", "Scope test"],
            "configure source repository name",
        )
        .unwrap();
        fs::write(repository.path().join("README.md"), "pinned run source").unwrap();
        run_git(
            Some(repository.path()),
            &["add", "README.md"],
            "stage source file",
        )
        .unwrap();
        run_git(
            Some(repository.path()),
            &["commit", "-m", "pin source"],
            "commit source file",
        )
        .unwrap();

        let pushed = git_push_from_repo(&state, repository.path(), None)
            .await
            .unwrap();
        let source = RunSource::accepted_git_head(
            "owner/repo",
            GitHead {
                head_oid: pushed.head.head_oid.clone(),
                push_sequence: pushed.head.push_sequence,
                change_version: 1,
                manifest: pushed.head.manifest.clone(),
            },
            vec![pushed.pack_span],
            ProjectionViewKey::Private,
        )
        .unwrap();

        let materialized = materialize_run_source_bundle(&state, &source, 4 * 1024 * 1024).unwrap();
        assert!(!materialized.bytes.is_empty());
        assert_eq!(
            materialized.sha256,
            hex::encode(Sha256::digest(&materialized.bytes))
        );

        let bundle = repository.path().join("source.bundle");
        fs::write(&bundle, materialized.bytes).unwrap();
        let output = std::process::Command::new("git")
            .args(["bundle", "list-heads", bundle.to_str().unwrap()])
            .output()
            .unwrap();
        assert!(output.status.success());
        assert!(String::from_utf8_lossy(&output.stdout).contains(&pushed.head.head_oid));
    }
}
