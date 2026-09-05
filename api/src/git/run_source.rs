pub(super) mod operation;

use crate::{
    error::ApiError,
    git::{restore::restore_git_pack_spans, upload::git_process_output_with_limits},
    persistence::ensure_private_dir,
    state::AppState,
};
use scope_domain::runs::source::RunSource;
use scope_domain::runs::workflow::identity::WorkflowPath;
use scope_git::DEFAULT_GIT_BRANCH;
use scope_object_store::source_blob_bytes_bounded;
use sha2::{Digest as _, Sha256};
use std::{
    fs::{self, File, OpenOptions},
    io::Read as _,
    os::unix::{fs::DirBuilderExt as _, fs::OpenOptionsExt as _, process::CommandExt as _},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::atomic::{AtomicU64, Ordering},
    thread,
    time::{Duration, Instant},
};

static RUN_SOURCE_ATTEMPT: AtomicU64 = AtomicU64::new(1);
const GIT_INSPECTION_TIMEOUT: Duration = Duration::from_secs(30);

pub(crate) struct MaterializedRunSource {
    pub(crate) bytes: Vec<u8>,
    pub(crate) sha256: String,
}

pub(crate) async fn materialize_run_source_bundle(
    state: &AppState,
    source: &RunSource,
    max_bytes: usize,
) -> Result<MaterializedRunSource, ApiError> {
    let bytes = if let Some(bundle) = source.ephemeral_bundle() {
        let object_store = state.object_store.clone();
        let bundle = bundle.clone();
        tokio::task::spawn_blocking(move || {
            source_blob_bytes_bounded(object_store.as_ref(), &bundle, max_bytes)
                .map_err(ApiError::from)
        })
        .await
        .map_err(|error| {
            ApiError::internal_message(format!("run source object read task failed: {error}"))
        })??
    } else {
        materialize_accepted_git_head_bundle(state, source, max_bytes).await?
    };
    Ok(MaterializedRunSource {
        sha256: hex::encode(Sha256::digest(&bytes)),
        bytes,
    })
}

async fn materialize_accepted_git_head_bundle(
    state: &AppState,
    source: &RunSource,
    max_bytes: usize,
) -> Result<Vec<u8>, ApiError> {
    let owner = operation::RunSourceOperation::new(state)?;
    let state = state.clone();
    let source = source.clone();
    operation::supervise(async move {
        materialize_owned_git_head_bundle(&state, &source, max_bytes, owner).await
    })
    .await
}

async fn materialize_owned_git_head_bundle(
    state: &AppState,
    source: &RunSource,
    max_bytes: usize,
    owner: std::sync::Arc<operation::RunSourceOperation>,
) -> Result<Vec<u8>, ApiError> {
    let (repository_id, head, pack_spans) = source.logical_git_head().ok_or_else(|| {
        ApiError::internal_message("run source does not contain a materializable Git head")
    })?;
    let repo = operation::repository(&owner);
    restore_git_pack_spans(state, repository_id, head, pack_spans, &repo, Some(&owner)).await?;
    let main_ref = format!("refs/heads/{DEFAULT_GIT_BRANCH}");
    let repo_path = repo;
    let timeout = state.runtime_budgets.git_command_timeout();
    let output = operation::spawn_blocking(Some(&owner), move || {
        git_process_output_with_limits(
            Command::new("git")
                .arg("--git-dir")
                .arg(repo_path)
                .args(["bundle", "create", "-", &main_ref]),
            None,
            timeout,
            max_bytes,
        )
    })
    .await
    .map_err(|error| {
        ApiError::internal_message(format!("run source Git bundle task failed: {error}"))
    })??;
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

pub(crate) fn inspect_manual_run_bundle(
    root: &Path,
    bytes: &[u8],
    git_oid: &str,
    workflow_name: &str,
) -> Result<scope_run_config::ParsedWorkflow, ApiError> {
    let yml = WorkflowPath::parse(format!("/.scope/runs/{workflow_name}.yml"))
        .map_err(ApiError::bad_request)?;
    let yaml = WorkflowPath::parse(format!("/.scope/runs/{workflow_name}.yaml"))
        .map_err(ApiError::bad_request)?;
    let temp = ManualRunInspection::new(root)?;
    let bundle = temp.path.join("source.bundle");
    let bare = temp.path.join("source.git");
    write_private_file(&bundle, bytes)?;
    let mut clone = Command::new("git");
    clone
        .args(["clone", "--bare", "--no-local"])
        .arg(&bundle)
        .arg(&bare)
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    if !run_git_with_timeout(&mut clone, "Git bundle clone")? {
        return Err(ApiError::bad_request("invalid Git bundle"));
    }
    let mut commit = Command::new("git");
    commit
        .arg("--git-dir")
        .arg(&bare)
        .args(["cat-file", "-e", &format!("{git_oid}^{{commit}}")])
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    if !run_git_with_timeout(&mut commit, "Git commit inspection")? {
        return Err(ApiError::bad_request(
            "requested Git commit is not present in the bundle",
        ));
    }
    let yml_bytes = git_blob(
        &bare,
        git_oid,
        yml.as_str().trim_start_matches('/'),
        &temp.path.join("workflow-yml"),
    )?;
    let yaml_bytes = git_blob(
        &bare,
        git_oid,
        yaml.as_str().trim_start_matches('/'),
        &temp.path.join("workflow-yaml"),
    )?;
    let (path, workflow_bytes) = match (yml_bytes, yaml_bytes) {
        (Some(_), Some(_)) => {
            return Err(ApiError::bad_request(format!(
                "workflow {workflow_name:?} is defined by both .yml and .yaml"
            )));
        }
        (Some(bytes), None) => (yml, bytes),
        (None, Some(bytes)) => (yaml, bytes),
        (None, None) => {
            return Err(ApiError::not_found(format!(
                "workflow {workflow_name:?} was not found at commit {git_oid}"
            )));
        }
    };
    scope_run_config::parse_workflow(path.as_str(), &workflow_bytes).map_err(ApiError::bad_request)
}

fn git_blob(
    bare: &Path,
    git_oid: &str,
    path: &str,
    output_prefix: &Path,
) -> Result<Option<Vec<u8>>, ApiError> {
    let object = format!("{git_oid}:{path}");
    let size_path = output_prefix.with_extension("size");
    let size_file = create_private_file(&size_path)?;
    let mut size = Command::new("git");
    size.arg("--git-dir")
        .arg(bare)
        .args(["cat-file", "-s", &object])
        .stdout(Stdio::from(size_file))
        .stderr(Stdio::null());
    if !run_git_with_timeout(&mut size, "Git workflow size inspection")? {
        return Ok(None);
    }
    let size_text = read_bounded_file(&size_path, 64)?;
    let size = std::str::from_utf8(&size_text)
        .map_err(|_| ApiError::bad_request("Git reported an invalid workflow size"))?
        .trim()
        .parse::<usize>()
        .map_err(|_| ApiError::bad_request("Git reported an invalid workflow size"))?;
    if size > scope_run_config::MAX_WORKFLOW_DEFINITION_BYTES {
        return Err(ApiError::bad_request(format!(
            "workflow definition exceeds {} bytes",
            scope_run_config::MAX_WORKFLOW_DEFINITION_BYTES
        )));
    }

    let blob_path = output_prefix.with_extension("blob");
    let blob_file = create_private_file(&blob_path)?;
    let mut blob = Command::new("git");
    blob.arg("--git-dir")
        .arg(bare)
        .args(["cat-file", "blob", &object])
        .stdout(Stdio::from(blob_file))
        .stderr(Stdio::null());
    if !run_git_with_timeout(&mut blob, "Git workflow read")? {
        return Err(ApiError::bad_request(
            "workflow changed while inspecting the Git bundle",
        ));
    }
    let bytes = read_bounded_file(&blob_path, scope_run_config::MAX_WORKFLOW_DEFINITION_BYTES)?;
    if bytes.len() != size {
        return Err(ApiError::bad_request(
            "Git workflow size changed while inspecting the bundle",
        ));
    }
    Ok(Some(bytes))
}

fn run_git_with_timeout(command: &mut Command, operation: &str) -> Result<bool, ApiError> {
    command.process_group(0);
    let mut child = command.spawn().map_err(ApiError::internal)?;
    let started = Instant::now();
    loop {
        if let Some(status) = child.try_wait().map_err(ApiError::internal)? {
            return Ok(status.success());
        }
        if started.elapsed() >= GIT_INSPECTION_TIMEOUT {
            let _ = Command::new("kill")
                .args(["-KILL", "--"])
                .arg(format!("-{}", child.id()))
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status();
            let _ = child.kill();
            let _ = child.wait();
            return Err(ApiError::bad_request(format!("{operation} timed out")));
        }
        thread::sleep(Duration::from_millis(25));
    }
}

fn write_private_file(path: &Path, bytes: &[u8]) -> Result<(), ApiError> {
    let mut file = create_private_file(path)?;
    std::io::Write::write_all(&mut file, bytes).map_err(ApiError::internal)
}

fn create_private_file(path: &Path) -> Result<File, ApiError> {
    OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(path)
        .map_err(ApiError::internal)
}

fn read_bounded_file(path: &Path, max_bytes: usize) -> Result<Vec<u8>, ApiError> {
    let file = File::open(path).map_err(ApiError::internal)?;
    let length = file.metadata().map_err(ApiError::internal)?.len();
    if length > max_bytes as u64 {
        return Err(ApiError::bad_request(format!(
            "Git command output exceeds {max_bytes} bytes"
        )));
    }
    let mut bytes = Vec::with_capacity(length as usize);
    file.take(max_bytes as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(ApiError::internal)?;
    if bytes.len() > max_bytes {
        return Err(ApiError::bad_request(format!(
            "Git command output exceeds {max_bytes} bytes"
        )));
    }
    Ok(bytes)
}

struct ManualRunInspection {
    path: PathBuf,
}

impl ManualRunInspection {
    fn new(root: &Path) -> Result<Self, ApiError> {
        fs::create_dir_all(root).map_err(ApiError::internal)?;
        for _ in 0..8 {
            let path = root.join(crate::persistence_ids::generate_prefixed_id("inspect_")?);
            match fs::DirBuilder::new().mode(0o700).create(&path) {
                Ok(()) => return Ok(Self { path }),
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
                Err(error) => return Err(ApiError::internal(error)),
            }
        }
        Err(ApiError::internal_message(
            "could not allocate run bundle inspection directory",
        ))
    }
}

impl Drop for ManualRunInspection {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::git::import::{git_push_from_repo, run_git, run_git_output};
    use scope_domain::{
        account::UserAccount, policy::Visibility, projection::ProjectionViewKey,
        repository::git::GitHead, runs::source::RunSource,
    };

    async fn git_head_fixture(state: &AppState) -> (RunSource, TemporaryRunSourceRepository) {
        let owner = UserAccount {
            id: "user-owner".to_string(),
            handle: "owner".to_string(),
            email: "owner@example.test".to_string(),
            email_verified: true,
        };
        let mut catalog = scope_postgres::db::CatalogFixture::default();
        catalog
            .create_repository(&owner, "repo", Visibility::Private)
            .unwrap();
        catalog.users.insert(owner.id.clone(), owner);
        state
            .metadata
            .admin()
            .seed_catalog_for_tests(catalog)
            .unwrap();
        let repository = TemporaryRunSourceRepository::new(state).unwrap();
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

        let pushed = git_push_from_repo(state, "owner/repo", repository.path(), None)
            .await
            .unwrap();
        let source = RunSource::accepted_git_head(
            "owner/repo",
            GitHead {
                head_oid: pushed.stored.head.head_oid.clone(),
                push_sequence: pushed.stored.head.push_sequence,
                change_version: 1,
                manifest: pushed.stored.head.manifest.clone(),
            },
            vec![pushed.stored.pack_span.clone()],
            ProjectionViewKey::Private,
        )
        .unwrap();

        state
            .git_segment_store
            .cleanup_local("owner/repo", &pushed.stored.pack_span.segment.segment_id)
            .await
            .unwrap();
        (source, repository)
    }

    #[tokio::test]
    async fn concurrent_git_head_materializations_restore_remote_segments_on_the_api_runtime() {
        let state = AppState::test_state();
        let (source, repository) = git_head_fixture(&state).await;
        let (first, second) = tokio::join!(
            materialize_run_source_bundle(&state, &source, 4 * 1024 * 1024),
            materialize_run_source_bundle(&state, &source, 4 * 1024 * 1024),
        );
        let materialized = first.unwrap();
        let concurrent = second.unwrap();
        assert!(!materialized.bytes.is_empty());
        assert_eq!(concurrent.bytes, materialized.bytes);
        assert_eq!(concurrent.sha256, materialized.sha256);
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
        assert!(
            String::from_utf8_lossy(&output.stdout)
                .contains(&source.logical_git_head().unwrap().1.head_oid)
        );
    }

    #[tokio::test]
    async fn cancelled_index_and_bundle_requests_keep_repository_and_capacity_until_exit() {
        use std::sync::{Arc, Mutex, atomic::AtomicUsize, mpsc};

        // Restore children are cleanup, init, index, update-ref, fsck, symbolic-ref,
        // followed by bundle creation. Pause inside the selected blocking child.
        for phase in [3, 7] {
            for outcome in ["success", "failure", "panic"] {
                let state = AppState::test_state();
                let (source, _fixture) = git_head_fixture(&state).await;
                let _other_permit = state.runtime_budgets.try_git_materialization().unwrap();
                let (started_tx, started_rx) = tokio::sync::oneshot::channel();
                let started_tx = Mutex::new(Some(started_tx));
                let (release_tx, release_rx) = mpsc::channel();
                let release_rx = Mutex::new(release_rx);
                let count = AtomicUsize::new(0);
                let repo_path = Arc::new(Mutex::new(PathBuf::new()));
                let path_for_hook = repo_path.clone();
                let owner = operation::with_hook(
                    &state,
                    Box::new(move || {
                        if count.fetch_add(1, Ordering::SeqCst) + 1 != phase {
                            return;
                        }
                        started_tx.lock().unwrap().take().unwrap().send(()).unwrap();
                        release_rx.lock().unwrap().recv().unwrap();
                        let path = path_for_hook.lock().unwrap();
                        assert!(path.exists(), "repository removed beneath blocking child");
                        match outcome {
                            "panic" => panic!("injected blocking child panic"),
                            "failure" => {
                                if phase == 3 {
                                    for entry in fs::read_dir(&*path).unwrap() {
                                        let entry = entry.unwrap();
                                        if entry
                                            .file_name()
                                            .to_string_lossy()
                                            .ends_with(".pack.tmp")
                                        {
                                            fs::remove_file(entry.path()).unwrap();
                                        }
                                    }
                                } else {
                                    fs::remove_file(path.join("refs/heads/main")).unwrap();
                                }
                            }
                            _ => {}
                        }
                    }),
                );
                // Observe the path without keeping the operation resources alive.
                let path = operation::repository(&owner);
                *repo_path.lock().unwrap() = path.clone();
                let operation_state = state.clone();
                let (completed_tx, completed_rx) = tokio::sync::oneshot::channel();
                let request = tokio::spawn(async move {
                    operation::supervise(async move {
                        let result = materialize_owned_git_head_bundle(
                            &operation_state,
                            &source,
                            4 * 1024 * 1024,
                            owner,
                        )
                        .await;
                        completed_tx.send(result.is_ok()).unwrap();
                        result
                    })
                    .await
                });
                tokio::time::timeout(Duration::from_secs(10), started_rx)
                    .await
                    .unwrap()
                    .unwrap();
                request.abort();
                assert!(request.await.unwrap_err().is_cancelled());
                assert!(path.exists());
                assert!(state.runtime_budgets.try_git_materialization().is_err());
                release_tx.send(()).unwrap();
                assert_eq!(
                    tokio::time::timeout(Duration::from_secs(10), completed_rx)
                        .await
                        .unwrap()
                        .unwrap(),
                    outcome == "success",
                );
                tokio::time::timeout(Duration::from_secs(10), async {
                    loop {
                        if let Ok(permit) = state.runtime_budgets.try_git_materialization() {
                            assert!(
                                !path.exists(),
                                "capacity returned before repository cleanup"
                            );
                            drop(permit);
                            break;
                        }
                        tokio::task::yield_now().await;
                    }
                })
                .await
                .unwrap();
            }
        }
    }

    #[test]
    fn manual_bundle_inspection_reads_the_requested_workflow_at_the_pinned_commit() {
        let state = AppState::test_state();
        let source = tempfile::tempdir().unwrap();
        run_git(
            None,
            &["init", "-b", "main", source.path().to_str().unwrap()],
            "initialize manual run source",
        )
        .unwrap();
        run_git(
            Some(source.path()),
            &["config", "user.email", "scope@test.invalid"],
            "configure source repository email",
        )
        .unwrap();
        run_git(
            Some(source.path()),
            &["config", "user.name", "Scope test"],
            "configure source repository name",
        )
        .unwrap();
        fs::create_dir_all(source.path().join(".scope/runs")).unwrap();
        fs::write(
            source.path().join(".scope/runs/checks.yml"),
            format!(
                "name: Checks\non:\n  manual: true\ncaches: []\ncontainer:\n  image: alpine@sha256:{}\ntimeout: 5m\njobs:\n  checks:\n    steps:\n      - name: Test\n        run: 'true'\n",
                "a".repeat(64)
            ),
        )
        .unwrap();
        run_git(
            Some(source.path()),
            &["add", "."],
            "stage manual run source",
        )
        .unwrap();
        run_git(
            Some(source.path()),
            &["commit", "-m", "manual run source"],
            "commit manual run source",
        )
        .unwrap();
        let git_oid = String::from_utf8(
            run_git_output(
                Some(source.path()),
                &["rev-parse", "HEAD"],
                "read manual run commit",
            )
            .unwrap()
            .stdout,
        )
        .unwrap();
        let git_oid = git_oid.trim();
        let bundle_path = source.path().join("source.bundle");
        run_git(
            Some(source.path()),
            &["bundle", "create", bundle_path.to_str().unwrap(), "HEAD"],
            "create manual run bundle",
        )
        .unwrap();

        let parsed = inspect_manual_run_bundle(
            &state.data_dir.join("manual-run-inspection-test"),
            &fs::read(bundle_path).unwrap(),
            git_oid,
            "checks",
        )
        .unwrap();
        let revision = parsed.into_revision("owner/repo").unwrap();

        assert!(revision.definition().triggers().manual());
        assert_eq!(
            revision.workflow().path().as_str(),
            "/.scope/runs/checks.yml"
        );
    }
}
