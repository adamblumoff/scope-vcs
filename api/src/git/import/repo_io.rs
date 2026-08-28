use crate::{
    config::{DEFAULT_GIT_BRANCH, MAX_PENDING_IMPORT_BLOB_BYTES, MAX_PENDING_IMPORT_FILES},
    error::ApiError,
    git::upload::{
        git_process_output_with_limits, git_process_output_with_timeout, truncated_git_stderr,
    },
    runtime_budgets::RuntimeBudgets,
    state::AppState,
};
use scope_domain::content::{SourceBlob, is_supported_git_file_mode};
use scope_domain::repository::git::GitHead;
use scope_domain::{
    policy::ScopePath,
    repo_control::{RepoControlPath, classify_repo_control_path},
};
use scope_git::{GitTreePath, StoredGitPush, prepare_git_push};
use scope_git_process::{ProcessLimits, StreamingProcessError, run_with_stdout};
use scope_git_storage::{ENCODING_VERSION, StagedGitSegment};
use scope_object_store::{ContentObjectKind, content_object_for_bytes, object_key};
use scope_postgres::db::ContentRefFence;
use sha2::{Digest, Sha256};
use std::{
    path::Path as FsPath,
    process::Command,
    time::{Duration, Instant},
};

pub(super) fn pushed_commit_message(
    staging_repo: &FsPath,
    head_oid: &str,
) -> Result<String, ApiError> {
    let message = git_stdout_text(
        staging_repo,
        &["log", "-1", "--format=%B", head_oid],
        "reading pushed commit message",
    )?;
    let message = message.trim_end_matches(&['\r', '\n'][..]).to_string();
    if message.trim().is_empty() {
        Ok(format!("Push to {DEFAULT_GIT_BRANCH}"))
    } else {
        Ok(message)
    }
}

pub(crate) fn git_refs(staging_repo: &FsPath) -> Result<Vec<(String, String)>, ApiError> {
    let main_ref = format!("refs/heads/{DEFAULT_GIT_BRANCH}");
    let output = run_git_output(
        Some(staging_repo),
        &[
            "for-each-ref",
            "--format=%(refname)%00%(objectname)",
            &main_ref,
            "refs/tags",
        ],
        "reading pushed refs",
    )?;
    if !output.status.success() {
        return Err(ApiError::infrastructure_unavailable(format!(
            "reading pushed refs: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        )));
    }
    let text = String::from_utf8(output.stdout).map_err(ApiError::bad_request)?;
    text.lines()
        .map(|line| {
            let (refname, oid) = line
                .split_once('\0')
                .ok_or_else(|| ApiError::internal_message("invalid git ref listing"))?;
            Ok((refname.to_string(), oid.to_string()))
        })
        .collect()
}

pub(super) fn describe_refs(refs: &[(String, String)]) -> String {
    if refs.is_empty() {
        return "none".to_string();
    }

    refs.iter()
        .map(|(name, oid)| format!("{name}@{}", oid.get(..12).unwrap_or(oid)))
        .collect::<Vec<_>>()
        .join(", ")
}

pub(super) fn git_tree_entries(
    staging_repo: &FsPath,
    head_oid: &str,
) -> Result<Vec<GitTreeFile>, ApiError> {
    git_tree_entries_for_path(staging_repo, head_oid, None, true)
}

pub(super) fn git_tree_entries_under(
    staging_repo: &FsPath,
    head_oid: &str,
    path: &str,
) -> Result<Vec<GitTreeFile>, ApiError> {
    git_tree_entries_for_path(staging_repo, head_oid, Some(path), false)
}

fn git_tree_entries_for_path(
    staging_repo: &FsPath,
    head_oid: &str,
    path: Option<&str>,
    enforce_import_limits: bool,
) -> Result<Vec<GitTreeFile>, ApiError> {
    let mut args = vec!["ls-tree", "-rz", "-r", "-l", head_oid];
    if let Some(path) = path {
        args.extend(["--", path]);
    }
    let output = run_git_output(Some(staging_repo), &args, "reading pushed tree")?;
    let mut pending_files = Vec::new();
    for raw in output.stdout.split(|byte| *byte == 0) {
        if raw.is_empty() {
            continue;
        }
        if enforce_import_limits && pending_files.len() >= MAX_PENDING_IMPORT_FILES {
            return Err(ApiError::bad_request(format!(
                "pending import exceeds {MAX_PENDING_IMPORT_FILES} files"
            )));
        }
        let entry = std::str::from_utf8(raw).map_err(ApiError::bad_request)?;
        let Some((metadata, path)) = entry.split_once('\t') else {
            return Err(ApiError::internal_message("invalid git tree entry"));
        };
        let mut fields = metadata.split_whitespace();
        let mode = fields
            .next()
            .ok_or_else(|| ApiError::internal_message("tree entry is missing mode"))?;
        let kind = fields
            .next()
            .ok_or_else(|| ApiError::internal_message("tree entry is missing type"))?;
        let oid = fields
            .next()
            .ok_or_else(|| ApiError::internal_message("tree entry is missing oid"))?;
        if kind != "blob" {
            return Err(ApiError::bad_request(format!(
                "unsupported Git tree entry {path}: {kind}"
            )));
        }
        let size = fields
            .next()
            .ok_or_else(|| ApiError::internal_message("tree entry is missing size"))?;
        let path = validate_pushed_file_path(path)?;
        if !is_supported_git_file_mode(mode) {
            return Err(ApiError::bad_request(format!(
                "unsupported Git file mode {path}: {mode}"
            )));
        }
        let blob_size = size
            .parse::<usize>()
            .map_err(|_| ApiError::internal_message("invalid Git blob size"))?;
        if enforce_import_limits && blob_size > MAX_PENDING_IMPORT_BLOB_BYTES {
            return Err(ApiError::bad_request(format!(
                "blob {path} is larger than {MAX_PENDING_IMPORT_BLOB_BYTES} bytes"
            )));
        }
        pending_files.push(GitTreeFile {
            path,
            mode: mode.to_string(),
            oid: oid.to_string(),
            size_bytes: blob_size,
        });
    }

    Ok(pending_files)
}

pub(super) fn git_changed_tree_entries(
    staging_repo: &FsPath,
    base_oid: Option<&str>,
    head_oid: &str,
) -> Result<Vec<(ScopePath, Option<GitTreeFile>)>, ApiError> {
    let Some(base_oid) = base_oid else {
        return git_tree_entries(staging_repo, head_oid)?
            .into_iter()
            .map(|entry| Ok((entry.path.to_scope_path(), Some(entry))))
            .collect();
    };
    let output = run_git_output(
        Some(staging_repo),
        &[
            "diff-tree",
            "--no-commit-id",
            "--raw",
            "-r",
            "-z",
            "--no-renames",
            base_oid,
            head_oid,
        ],
        "reading pushed Git delta",
    )?;
    if !output.status.success() {
        return Err(ApiError::infrastructure_unavailable(format!(
            "reading pushed Git delta: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        )));
    }

    let mut fields = output.stdout.split(|byte| *byte == 0);
    let mut pending = Vec::new();
    while let Some(header) = fields.next() {
        if header.is_empty() {
            continue;
        }
        let path = fields
            .next()
            .ok_or_else(|| ApiError::internal_message("Git delta is missing a path"))?;
        let header = std::str::from_utf8(header).map_err(ApiError::bad_request)?;
        let path = std::str::from_utf8(path).map_err(ApiError::bad_request)?;
        let path = validate_pushed_file_path(path)?;
        let values = header.split_whitespace().collect::<Vec<_>>();
        if values.len() != 5 || !values[0].starts_with(':') {
            return Err(ApiError::internal_message("invalid Git delta record"));
        }
        let new_mode = values[1];
        let new_oid = values[3];
        let status = values[4];
        let scope_path = path.to_scope_path();
        if status == "D" {
            pending.push((scope_path, None));
            continue;
        }
        if !is_supported_git_file_mode(new_mode) {
            return Err(ApiError::bad_request(format!(
                "unsupported Git file mode {path}: {new_mode}"
            )));
        }
        pending.push((
            scope_path,
            Some(GitTreeFile {
                path,
                mode: new_mode.to_string(),
                oid: new_oid.to_string(),
                size_bytes: 0,
            }),
        ));
    }

    let requested_oids = pending
        .iter()
        .filter_map(|(_, entry)| entry.as_ref().map(|entry| entry.oid.as_str()))
        .map(|oid| format!("{oid}\n"))
        .collect::<String>();
    if !requested_oids.is_empty() {
        let output = git_process_output_with_timeout(
            Command::new("git").current_dir(staging_repo).args([
                "cat-file",
                "--batch-check=%(objectname) %(objecttype) %(objectsize)",
            ]),
            Some(requested_oids.into_bytes()),
            RuntimeBudgets::default_git_command_timeout(),
        )?;
        if !output.status.success() {
            return Err(ApiError::infrastructure_unavailable(format!(
                "reading pushed blob sizes: {}",
                String::from_utf8_lossy(&output.stderr).trim()
            )));
        }
        let size_output = String::from_utf8(output.stdout).map_err(ApiError::bad_request)?;
        let mut sizes = size_output.lines().map(|line| {
            let values = line.split_whitespace().collect::<Vec<_>>();
            if values.len() != 3 || values[1] != "blob" {
                return Err(ApiError::bad_request("pushed path is not a Git blob"));
            }
            values[2]
                .parse::<usize>()
                .map_err(|_| ApiError::internal_message("invalid Git blob size"))
        });
        for (_, entry) in &mut pending {
            if let Some(entry) = entry {
                entry.size_bytes = sizes
                    .next()
                    .ok_or_else(|| ApiError::internal_message("missing Git blob size"))??;
                if entry.size_bytes > MAX_PENDING_IMPORT_BLOB_BYTES {
                    return Err(ApiError::bad_request(format!(
                        "blob {} is larger than {MAX_PENDING_IMPORT_BLOB_BYTES} bytes",
                        entry.path
                    )));
                }
            }
        }
    }
    if pending.len() > MAX_PENDING_IMPORT_FILES {
        return Err(ApiError::bad_request(format!(
            "pending import exceeds {MAX_PENDING_IMPORT_FILES} files"
        )));
    }
    Ok(pending)
}

pub(crate) fn validate_pushed_tree(staging_repo: &FsPath, head_oid: &str) -> Result<(), ApiError> {
    let entries = git_tree_entries(staging_repo, head_oid)?;
    // The server owns the canonical rules invariant. Agent-specific adapters depend on
    // repo-local tool signals and remain a `scope push` preflight concern.
    if !entries
        .iter()
        .any(|entry| entry.path.as_str() == ".scope/RULES.md")
    {
        return Err(ApiError::bad_request(
            "pushed main tree must contain .scope/RULES.md",
        ));
    }
    Ok(())
}

pub(crate) fn validate_pushed_commit_range(
    staging_repo: &FsPath,
    base_oid: Option<&str>,
    head_oid: &str,
) -> Result<(), ApiError> {
    let mut args = vec!["rev-list", "--reverse", head_oid];
    let excluded_base = base_oid.map(|oid| format!("^{oid}"));
    if let Some(excluded_base) = excluded_base.as_deref() {
        args.push(excluded_base);
    }
    let commits = git_stdout_text(staging_repo, &args, "reading pushed commit range")?;
    for commit_oid in commits.lines() {
        validate_pushed_tree(staging_repo, commit_oid)?;
    }
    Ok(())
}

pub(crate) struct FencedGitPush {
    pub(crate) stored: StoredGitPush,
    pub(crate) fence: ContentRefFence,
    pub(crate) staged_segment: StagedGitSegment,
    pub(crate) upload_heartbeat: GitSegmentUploadHeartbeat,
}

pub(crate) struct GitSegmentUploadHeartbeat {
    task: tokio::task::JoinHandle<()>,
}

impl GitSegmentUploadHeartbeat {
    pub(crate) fn start(state: &AppState, segment_id: String) -> Self {
        let metadata = state.metadata.clone();
        let task = tokio::spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_secs(60));
            interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
            loop {
                interval.tick().await;
                let now = match crate::persistence::unix_now() {
                    Ok(now) => now,
                    Err(error) => {
                        tracing::warn!(segment_id, error = %error.into_operator_diagnostic(), "Git segment upload heartbeat clock failed");
                        continue;
                    }
                };
                match metadata
                    .repositories()
                    .touch_git_segment_upload(&segment_id, now)
                    .await
                {
                    Ok(true) => {}
                    Ok(false) => return,
                    Err(error) => {
                        tracing::warn!(segment_id, error = %error.message, "Git segment upload heartbeat failed")
                    }
                }
            }
        });
        Self { task }
    }
}

impl Drop for GitSegmentUploadHeartbeat {
    fn drop(&mut self) {
        self.task.abort();
    }
}

pub(crate) async fn git_push_from_repo(
    state: &AppState,
    repository_id: &str,
    repo: &FsPath,
    previous: Option<&GitHead>,
) -> Result<FencedGitPush, ApiError> {
    let _ingest_permit = state.runtime_budgets.try_git_segment_ingest()?;
    let storage_limits = state.runtime_budgets.git_storage_limits();
    let refname = format!("refs/heads/{DEFAULT_GIT_BRANCH}");
    let head_oid = git_stdout_text(repo, &["rev-parse", &refname], "reading pushed Git head")?
        .trim()
        .to_string();
    let mut revisions = format!("{head_oid}\n");
    if let Some(previous) = previous {
        revisions.push('^');
        revisions.push_str(&previous.head_oid);
        revisions.push('\n');
    }
    let reservation = state
        .git_segment_store
        .reserve(repository_id)
        .map_err(|error| ApiError::infrastructure_unavailable(error.to_string()))?;
    let segment_id = reservation.segment_id.clone();
    let object_key = reservation.object_key.clone();
    state
        .metadata
        .repositories()
        .begin_git_segment_upload(
            repository_id,
            &segment_id,
            &object_key,
            ENCODING_VERSION,
            crate::persistence::unix_now()?,
        )
        .await?;
    let upload_heartbeat = GitSegmentUploadHeartbeat::start(state, segment_id.clone());

    let pack_started = Instant::now();
    let segment_store = state.git_segment_store.clone();
    let repository_id_for_ingest = repository_id.to_string();
    let repo = repo.to_path_buf();
    let timeout = state.runtime_budgets.git_command_timeout();
    let output = tokio::task::spawn_blocking(move || {
        let runtime = tokio::runtime::Handle::current();
        let mut command = Command::new("git");
        command
            .current_dir(repo)
            .args(["pack-objects", "--revs", "--stdout"]);
        run_with_stdout(
            &mut command,
            Some(revisions.into_bytes()),
            ProcessLimits::new(timeout),
            "creating incremental Git pack",
            move |stdout| {
                runtime.block_on(segment_store.ingest_reserved_blocking_reader(
                    &repository_id_for_ingest,
                    reservation,
                    stdout,
                ))
            },
        )
    })
    .await;
    let output = match output {
        Ok(output) => output,
        Err(error) => {
            best_effort_delete_git_segment_identity(state, repository_id, &segment_id, &object_key)
                .await;
            return Err(ApiError::internal(error));
        }
    };
    let pack_elapsed = pack_started.elapsed();
    let output = match output {
        Ok(output) => output,
        Err(error) => {
            best_effort_delete_git_segment_identity(state, repository_id, &segment_id, &object_key)
                .await;
            return Err(match error {
                StreamingProcessError::Process(error) => {
                    ApiError::infrastructure_unavailable(error.to_string())
                }
                StreamingProcessError::Consumer(error) => {
                    ApiError::infrastructure_unavailable(error.to_string())
                }
            });
        }
    };
    let staged_segment = output.value;
    if !output.status.success() {
        tracing::info!(
            incremental = previous.is_some(),
            pack_us = pack_elapsed.as_micros(),
            pack_bytes = staged_segment.segment.plaintext_bytes,
            success = false,
            "Git push pack timing"
        );
        best_effort_delete_staged_git_segment(state, repository_id, &staged_segment).await;
        return Err(ApiError::infrastructure_unavailable(format!(
            "creating incremental Git pack: {}",
            truncated_git_stderr(&output.stderr).trim()
        )));
    }
    if let Err(error) = state
        .metadata
        .repositories()
        .mark_git_segment_upload_ready(
            &staged_segment.segment,
            staged_segment.encrypted_bytes,
            crate::persistence::unix_now()?,
        )
        .await
    {
        best_effort_delete_staged_git_segment(state, repository_id, &staged_segment).await;
        return Err(error.into());
    }

    let pack_bytes = staged_segment.segment.plaintext_bytes;
    let store_started = Instant::now();
    let prepared = match prepare_git_push(
        staged_segment.segment.clone(),
        head_oid,
        previous,
        storage_limits,
    ) {
        Ok(prepared) => prepared,
        Err(error) => {
            best_effort_delete_staged_git_segment(state, repository_id, &staged_segment).await;
            return Err(error.into());
        }
    };
    let content_refs = [prepared.manifest().content_ref.clone()];
    let fence = match state
        .metadata
        .acquire_content_ref_fence(&content_refs)
        .await
    {
        Ok(fence) => fence,
        Err(error) => {
            best_effort_delete_staged_git_segment(state, repository_id, &staged_segment).await;
            return Err(error.into());
        }
    };
    let stored = prepared.store_manifest(state.object_store.as_ref());
    let timings = &staged_segment.timings;
    tracing::info!(
        phase = "complete",
        repository_id,
        segment_id = staged_segment.segment.segment_id,
        success = stored.is_ok(),
        duration_us = timings.total.as_micros(),
        bytes = timings.plaintext_bytes,
        blocked_us = timings.fanout_blocked.as_micros(),
        active_ingests = 1_u64,
        buffered_bytes = timings.chunk_bytes.saturating_mul(timings.channel_capacity),
        disk_free_bytes = disk_free_bytes(staged_segment.local_pack_path()),
        ledger_uploading = 0_u64,
        ledger_ready = u64::from(stored.is_ok()),
        ledger_published = 0_u64,
        orphan_count = u64::from(stored.is_err()),
        "Git segment ingest telemetry"
    );
    tracing::info!(
        incremental = previous.is_some(),
        pack_us = pack_elapsed.as_micros(),
        store_us = store_started.elapsed().as_micros(),
        pack_bytes,
        success = stored.is_ok(),
        "Git push pack timing"
    );
    match stored {
        Ok(stored) => Ok(FencedGitPush {
            stored,
            fence,
            staged_segment,
            upload_heartbeat,
        }),
        Err(error) => {
            fence.release().await;
            best_effort_delete_staged_git_segment(state, repository_id, &staged_segment).await;
            Err(error.into())
        }
    }
}

#[cfg(unix)]
fn disk_free_bytes(path: &FsPath) -> u64 {
    use std::{ffi::CString, os::unix::ffi::OsStrExt};

    let Some(path) = path.parent() else {
        return 0;
    };
    let Ok(path) = CString::new(path.as_os_str().as_bytes()) else {
        return 0;
    };
    let mut stats = std::mem::MaybeUninit::<libc::statvfs>::uninit();
    // SAFETY: `path` is a live NUL-terminated string and `stats` points to
    // writable storage initialized by statvfs on success.
    if unsafe { libc::statvfs(path.as_ptr(), stats.as_mut_ptr()) } != 0 {
        return 0;
    }
    // SAFETY: statvfs returned success and initialized the structure.
    let stats = unsafe { stats.assume_init() };
    stats.f_bavail.saturating_mul(stats.f_frsize)
}

#[cfg(not(unix))]
fn disk_free_bytes(_path: &FsPath) -> u64 {
    0
}

pub(crate) async fn best_effort_delete_staged_git_segment(
    state: &AppState,
    repository_id: &str,
    staged: &StagedGitSegment,
) {
    best_effort_delete_git_segment_identity(
        state,
        repository_id,
        &staged.segment.segment_id,
        &staged.object_key,
    )
    .await;
    if let Err(error) = state.git_segment_store.delete_local(staged).await {
        tracing::warn!(
            repository_id,
            segment_id = staged.segment.segment_id,
            error = %error,
            "failed to delete local Git segment"
        );
    }
}

async fn best_effort_delete_git_segment_identity(
    state: &AppState,
    repository_id: &str,
    segment_id: &str,
    object_key: &str,
) {
    let now = crate::persistence::unix_now().unwrap_or(0);
    let abandoned = match state
        .metadata
        .repositories()
        .abandon_git_segment_upload(segment_id, now)
        .await
    {
        Ok(abandoned) => abandoned,
        Err(error) => {
            tracing::warn!(repository_id, segment_id, error = %error.message, "failed to abandon Git segment");
            return;
        }
    };
    if !abandoned {
        tracing::warn!(
            repository_id,
            segment_id,
            "Git segment may already be published; leaving remote bytes untouched"
        );
        return;
    }
    if let Err(error) = state.git_segment_store.cleanup_remote(object_key).await {
        tracing::warn!(repository_id, segment_id, error = %error, "failed to delete remote Git segment");
        return;
    }
    if let Err(error) = state
        .metadata
        .repositories()
        .mark_git_segment_upload_deleted(segment_id, crate::persistence::unix_now().unwrap_or(now))
        .await
    {
        tracing::warn!(repository_id, segment_id, error = %error.message, "failed to mark Git segment deleted");
    }
}

pub(super) async fn queue_failed_git_objects(
    state: &AppState,
    objects: Vec<SourceBlob>,
) -> Result<(), ApiError> {
    if objects.is_empty() {
        return Ok(());
    }
    match state
        .metadata
        .cleanup()
        .queue_pending_source_blob_deletions(
            objects.clone(),
            crate::persistence::unix_now()?,
            &crate::persistence_ids::generate_persistence_id,
        )
        .await
    {
        Ok(()) => Ok(()),
        Err(queue_error) => {
            for object in objects {
                if let Err(delete_error) = state.object_store.delete(&object_key(&object)) {
                    return Err(ApiError::infrastructure_unavailable(format!(
                        "failed to queue or delete incomplete Git object: {}; {}",
                        queue_error.message, delete_error.message
                    )));
                }
            }
            Ok(())
        }
    }
}

pub(crate) fn git_snapshot_from_ref(
    repo: &FsPath,
    refname: &str,
) -> Result<(SourceBlob, Vec<u8>), ApiError> {
    git_snapshot_from_refs(repo, &[refname.to_string()])
}

fn git_snapshot_from_refs(
    repo: &FsPath,
    refs: &[String],
) -> Result<(SourceBlob, Vec<u8>), ApiError> {
    let [refname] = refs else {
        return Err(ApiError::internal_message(
            "Git snapshots must contain exactly one ref",
        ));
    };
    let head_oid = git_stdout_text(repo, &["rev-parse", refname], "reading Git snapshot head")?;
    let bundle_path = repo.join(format!("scope-snapshot-{}.bundle", random_bundle_id()?));
    let bundle = bundle_path.to_string_lossy().to_string();
    let mut args = vec!["bundle", "create", bundle.as_str()];
    args.extend(refs.iter().map(String::as_str));
    run_git(Some(repo), &args, "creating Git snapshot bundle")?;
    let bytes = std::fs::read(&bundle_path).map_err(ApiError::internal)?;
    let _ = std::fs::remove_file(&bundle_path);
    let mut snapshot = content_object_for_bytes(ContentObjectKind::GitBundle, &bytes);
    snapshot.git_oid = head_oid.trim().to_string();
    Ok((snapshot, bytes))
}

fn random_bundle_id() -> Result<String, ApiError> {
    let mut bytes = [0_u8; 16];
    getrandom::fill(&mut bytes).map_err(|error| {
        ApiError::internal_message(format!("Git snapshot bundle id generation failed: {error}"))
    })?;
    Ok(format!("{}-{}", std::process::id(), hex::encode(bytes)))
}

#[derive(Debug)]
pub(crate) struct GitTreeFile {
    pub(crate) path: GitTreePath,
    pub(crate) mode: String,
    pub(crate) oid: String,
    pub(crate) size_bytes: usize,
}

pub(crate) fn validate_pushed_file_path(path: &str) -> Result<GitTreePath, ApiError> {
    let path = GitTreePath::parse(path).map_err(ApiError::bad_request)?;
    let scope_path = path.to_scope_path();
    if matches!(
        classify_repo_control_path(&scope_path),
        Some(RepoControlPath::Forbidden)
    ) {
        return Err(ApiError::bad_request(format!(
            "Scope control path {:?} is not a supported tracked control file",
            path.as_str()
        )));
    }

    Ok(path)
}

pub(crate) fn run_git(repo: Option<&FsPath>, args: &[&str], action: &str) -> Result<(), ApiError> {
    let output = run_git_output(repo, args, action)?;
    if output.status.success() {
        Ok(())
    } else {
        Err(ApiError::infrastructure_unavailable(format!(
            "{action}: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        )))
    }
}

pub(crate) fn git_stdout_text(
    repo: &FsPath,
    args: &[&str],
    action: &str,
) -> Result<String, ApiError> {
    let output = run_git_output(Some(repo), args, action)?;
    if !output.status.success() {
        return Err(ApiError::infrastructure_unavailable(format!(
            "{action}: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        )));
    }
    String::from_utf8(output.stdout).map_err(ApiError::bad_request)
}

pub(crate) fn run_git_output(
    repo: Option<&FsPath>,
    args: &[&str],
    action: &str,
) -> Result<std::process::Output, ApiError> {
    let mut command = Command::new("git");
    if let Some(repo) = repo {
        command.arg("-C").arg(repo);
    }
    command.args(args);
    git_process_output_with_timeout(
        &mut command,
        None,
        RuntimeBudgets::default_git_command_timeout(),
    )
    .map_err(|error| {
        ApiError::infrastructure_unavailable(format!(
            "failed {action}: {}",
            error.operator_diagnostic()
        ))
    })
}

pub(crate) fn run_git_output_bounded(
    repo: Option<&FsPath>,
    args: &[&str],
    action: &str,
    max_stdout_bytes: usize,
) -> Result<std::process::Output, ApiError> {
    let mut command = Command::new("git");
    if let Some(repo) = repo {
        command.arg("-C").arg(repo);
    }
    command.args(args);
    git_process_output_with_limits(
        &mut command,
        None,
        RuntimeBudgets::default_git_command_timeout(),
        max_stdout_bytes,
    )
    .map_err(|error| match error.status() {
        axum::http::StatusCode::PAYLOAD_TOO_LARGE => {
            ApiError::payload_too_large(format!("{action} exceeded {max_stdout_bytes} bytes"))
        }
        _ => error,
    })
}

pub(crate) fn safe_repo_key(owner: &str, repo_name: &str) -> String {
    let repo_id = scope_domain::repository::repo_id(owner, repo_name);
    let digest = Sha256::digest(repo_id.as_bytes());
    format!("repo-{digest:x}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        fs,
        time::{SystemTime, UNIX_EPOCH},
    };

    #[test]
    fn bounded_git_output_rejects_stdout_over_the_limit() {
        let error =
            run_git_output_bounded(None, &["--version"], "reading Git version", 1).unwrap_err();
        assert_eq!(error.status(), axum::http::StatusCode::PAYLOAD_TOO_LARGE);
    }

    #[test]
    fn workflow_tree_listing_is_scoped_to_the_runs_directory() {
        let root = std::env::temp_dir().join(format!(
            "scope-workflow-tree-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(root.join(".scope/runs")).unwrap();
        run_git(Some(&root), &["init", "-q"], "initializing test repository").unwrap();
        fs::write(root.join("README.md"), "unrelated\n").unwrap();
        fs::write(root.join(".scope/runs/test.yml"), "name: Test\n").unwrap();
        run_git(Some(&root), &["add", "."], "staging test repository").unwrap();
        run_git(
            Some(&root),
            &[
                "-c",
                "user.name=Scope Tests",
                "-c",
                "user.email=scope@example.com",
                "commit",
                "-qm",
                "test",
            ],
            "committing test repository",
        )
        .unwrap();
        let head = git_stdout_text(&root, &["rev-parse", "HEAD"], "reading test head")
            .unwrap()
            .trim()
            .to_string();

        let entries = git_tree_entries_under(&root, &head, ".scope/runs").unwrap();

        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].path.as_str(), ".scope/runs/test.yml");
        fs::remove_dir_all(root).unwrap();
    }
}
