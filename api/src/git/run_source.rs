pub(super) mod operation;

use crate::{
    error::ApiError,
    git::{
        cache::GitDerivedCacheNamespace, restore::restore_git_pack_spans,
        upload::git_process_output_with_limits,
    },
    state::AppState,
};
use scope_domain::{
    repository::RepositoryIncarnation,
    runs::{run::Run, source::RunSource, workflow::identity::WorkflowPath},
};
use scope_git::DEFAULT_GIT_BRANCH;
use scope_object_store::source_blob_bytes_bounded;
use sha2::{Digest as _, Sha256};
use std::{
    fs::{self, File, OpenOptions},
    io::Read as _,
    os::unix::{fs::DirBuilderExt as _, fs::OpenOptionsExt as _, process::CommandExt as _},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    thread,
    time::{Duration, Instant},
};

const GIT_INSPECTION_TIMEOUT: Duration = Duration::from_secs(30);

pub(crate) struct MaterializedRunSource {
    pub(crate) bytes: Vec<u8>,
    pub(crate) sha256: String,
}

pub(crate) async fn materialize_run_source_bundle(
    state: &AppState,
    run: &Run,
    max_bytes: usize,
) -> Result<MaterializedRunSource, ApiError> {
    let source = &run.source;
    if let Some(bundle) = source.ephemeral_bundle() {
        let object_store = state.object_store.clone();
        let bundle = bundle.clone();
        return tokio::task::spawn_blocking(move || {
            let bytes = source_blob_bytes_bounded(object_store.as_ref(), &bundle, max_bytes)
                .map_err(ApiError::from)?;
            Ok(MaterializedRunSource {
                sha256: hex::encode(Sha256::digest(&bytes)),
                bytes,
            })
        })
        .await
        .map_err(|error| {
            ApiError::internal_message(format!("run source object read task failed: {error}"))
        })?;
    }
    let incarnation = state
        .metadata
        .repositories()
        .run_repository_incarnation(&run.id, run.workflow.repository_id())
        .await?
        .ok_or_else(|| ApiError::not_found("run repository not found"))?;
    materialize_accepted_git_head_bundle(state, &incarnation, source, max_bytes).await
}

fn run_source_cache_key(
    incarnation: &RepositoryIncarnation,
    source: &RunSource,
) -> Result<String, ApiError> {
    let identity = serde_json::to_vec(&(
        "run-source-bundle-v1",
        incarnation.repository_id(),
        incarnation.incarnation_id(),
        source,
    ))
    .map_err(ApiError::internal)?;
    Ok(format!(
        "run-source-{}",
        hex::encode(Sha256::digest(identity))
    ))
}

async fn materialize_accepted_git_head_bundle(
    state: &AppState,
    incarnation: &RepositoryIncarnation,
    source: &RunSource,
    max_bytes: usize,
) -> Result<MaterializedRunSource, ApiError> {
    let key = run_source_cache_key(incarnation, source)?;
    let path = state
        .repository_engine
        .cache_root()
        .join(format!("{key}.git"));
    let ready_path = path.clone();
    let build_path = path.clone();
    let build_state = state.clone();
    let build_source = source.clone();
    let handle = state
        .repository_engine
        .materialize_derived(
            incarnation,
            GitDerivedCacheNamespace::RunSource,
            key,
            &path,
            move || {
                ready_path.join("source.bundle").is_file() && ready_path.join("sha256").is_file()
            },
            move || build_accepted_source_bundle(build_state, build_source, build_path, max_bytes),
        )
        .await?;
    let read_permit = state
        .runtime_budgets
        .try_object_store("run source cache read")?;
    tokio::task::spawn_blocking(move || {
        let _read_permit = read_permit;
        let bytes = read_bounded_file(&handle.join("source.bundle"), max_bytes)?;
        let sha256 = String::from_utf8(read_bounded_file(&handle.join("sha256"), 64)?)
            .map_err(ApiError::internal)?;
        Ok(MaterializedRunSource { bytes, sha256 })
    })
    .await
    .map_err(|error| {
        ApiError::internal_message(format!("run source cache read task failed: {error}"))
    })?
}

async fn build_accepted_source_bundle(
    state: AppState,
    source: RunSource,
    path: PathBuf,
    max_bytes: usize,
) -> Result<(), ApiError> {
    operation::supervise(async move {
        let owner = operation::RunSourceOperation::new(&state)?;
        let bytes =
            materialize_owned_git_head_bundle(&state, &source, max_bytes, owner.clone()).await?;
        let temporary = TemporarySourceDirectory::new(state.repository_engine.cache_root())?;
        operation::spawn_blocking(Some(&owner), move || {
            write_private_file(&temporary.path.join("source.bundle"), &bytes)?;
            write_private_file(
                &temporary.path.join("sha256"),
                hex::encode(Sha256::digest(&bytes)).as_bytes(),
            )?;
            fs::rename(&temporary.path, path).map_err(ApiError::internal)
        })
        .await
        .map_err(|error| {
            ApiError::internal_message(format!("run source cache publication task failed: {error}"))
        })?
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
    let temp = TemporarySourceDirectory::new(root)?;
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

struct TemporarySourceDirectory {
    path: PathBuf,
}

impl TemporarySourceDirectory {
    fn path(&self) -> &Path {
        &self.path
    }

    fn new(root: &Path) -> Result<Self, ApiError> {
        fs::create_dir_all(root).map_err(ApiError::internal)?;
        for _ in 0..8 {
            let path = root.join(format!(
                "{}.tmp",
                crate::persistence_ids::generate_prefixed_id("inspect_")?
            ));
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

impl Drop for TemporarySourceDirectory {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

#[cfg(test)]
mod tests;
