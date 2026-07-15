use crate::domain::store::{GitHead, GitSegment, SourceBlob, is_supported_git_file_mode};
use crate::domain::{policy::ScopePath, repo_config::is_reserved_config_path};
use crate::{
    config::{
        DEFAULT_GIT_BRANCH, MAX_PENDING_IMPORT_BLOB_BYTES, MAX_PENDING_IMPORT_FILES,
        MAX_PENDING_IMPORT_TOTAL_BYTES,
    },
    error::ApiError,
    git::upload::git_process_output_with_timeout,
    object_store::put_repo_object,
    runtime_budgets::RuntimeBudgets,
    state::AppState,
};
use scope_core::git_segments::GitSegmentManifest;
use sha2::{Digest, Sha256};
use std::{path::Path as FsPath, process::Command};

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
        return Err(ApiError::service_unavailable(format!(
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
    let output = run_git_output(
        Some(staging_repo),
        &["ls-tree", "-rz", "-r", "-l", head_oid],
        "reading pushed tree",
    )?;
    let mut pending_files = Vec::new();
    let mut total_bytes = 0usize;
    for raw in output.stdout.split(|byte| *byte == 0) {
        if raw.is_empty() {
            continue;
        }
        if pending_files.len() >= MAX_PENDING_IMPORT_FILES {
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
        validate_pushed_file_path(path)?;
        if !is_supported_git_file_mode(mode) {
            return Err(ApiError::bad_request(format!(
                "unsupported Git file mode {path}: {mode}"
            )));
        }
        let blob_size = size
            .parse::<usize>()
            .map_err(|_| ApiError::internal_message("invalid Git blob size"))?;
        if blob_size > MAX_PENDING_IMPORT_BLOB_BYTES {
            return Err(ApiError::bad_request(format!(
                "blob {path} is larger than {MAX_PENDING_IMPORT_BLOB_BYTES} bytes"
            )));
        }
        total_bytes = total_bytes
            .checked_add(blob_size)
            .ok_or_else(|| ApiError::bad_request("pending import is too large"))?;
        if total_bytes > MAX_PENDING_IMPORT_TOTAL_BYTES {
            return Err(ApiError::bad_request(format!(
                "pending import exceeds {MAX_PENDING_IMPORT_TOTAL_BYTES} bytes"
            )));
        }
        pending_files.push(GitTreeFile {
            path: path.to_string(),
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
            .map(|entry| {
                let path =
                    ScopePath::parse(format!("/{}", entry.path)).map_err(ApiError::bad_request)?;
                Ok((path, Some(entry)))
            })
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
        return Err(ApiError::service_unavailable(format!(
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
        validate_pushed_file_path(path)?;
        let values = header.split_whitespace().collect::<Vec<_>>();
        if values.len() != 5 || !values[0].starts_with(':') {
            return Err(ApiError::internal_message("invalid Git delta record"));
        }
        let new_mode = values[1];
        let new_oid = values[3];
        let status = values[4];
        let scope_path = ScopePath::parse(format!("/{path}")).map_err(ApiError::bad_request)?;
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
                path: path.to_string(),
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
            return Err(ApiError::service_unavailable(format!(
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
    let total_bytes = pending
        .iter()
        .filter_map(|(_, entry)| entry.as_ref())
        .try_fold(0usize, |total, entry| total.checked_add(entry.size_bytes))
        .ok_or_else(|| ApiError::bad_request("pending import is too large"))?;
    if total_bytes > MAX_PENDING_IMPORT_TOTAL_BYTES {
        return Err(ApiError::bad_request(format!(
            "pending import exceeds {MAX_PENDING_IMPORT_TOTAL_BYTES} bytes"
        )));
    }
    Ok(pending)
}

pub(crate) fn validate_pushed_tree(staging_repo: &FsPath, head_oid: &str) -> Result<(), ApiError> {
    git_tree_entries(staging_repo, head_oid).map(|_| ())
}

pub(crate) struct CreatedGitSegment {
    pub(crate) head: GitHead,
    pub(crate) segment: GitSegment,
}

pub(crate) async fn git_segment_manifest_from_repo(
    state: &AppState,
    repo_id: &str,
    repo: &FsPath,
    previous: Option<&GitHead>,
) -> Result<CreatedGitSegment, ApiError> {
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
    let output = git_process_output_with_timeout(
        Command::new("git")
            .current_dir(repo)
            .args(["pack-objects", "--revs", "--stdout"]),
        Some(revisions.into_bytes()),
        state.runtime_budgets.git_command_timeout(),
    )?;
    if !output.status.success() {
        return Err(ApiError::service_unavailable(format!(
            "creating incremental Git segment: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        )));
    }
    let bytes = output.stdout;
    let segment = put_repo_object(state.object_store.as_ref(), repo_id, "git-segments", &bytes)?;
    let manifest = GitSegmentManifest::new(
        head_oid.clone(),
        previous.map(|head| head.manifest.clone()),
        segment.clone(),
    );
    let manifest_bytes = match manifest.encode() {
        Ok(bytes) => bytes,
        Err(error) => {
            queue_failed_segment(state, segment.clone()).await?;
            return Err(error.into());
        }
    };
    let mut snapshot = match put_repo_object(
        state.object_store.as_ref(),
        repo_id,
        "git-manifests",
        &manifest_bytes,
    ) {
        Ok(snapshot) => snapshot,
        Err(error) => {
            queue_failed_segment(state, segment.clone()).await?;
            return Err(error.into());
        }
    };
    snapshot.git_oid = head_oid.clone();
    let sequence = previous.map_or(1, |head| head.segment_sequence.saturating_add(1));
    Ok(CreatedGitSegment {
        head: GitHead {
            head_oid: head_oid.clone(),
            segment_sequence: sequence,
            change_version: previous.map_or(1, |head| head.change_version.saturating_add(1)),
            manifest: snapshot.clone(),
        },
        segment: GitSegment {
            sequence,
            base_oid: previous.map(|head| head.head_oid.clone()),
            head_oid,
            object: segment.clone(),
            manifest: snapshot,
        },
    })
}

async fn queue_failed_segment(state: &AppState, segment: SourceBlob) -> Result<(), ApiError> {
    match state
        .metadata
        .queue_pending_source_blob_deletions(vec![segment.clone()])
        .await
    {
        Ok(()) => Ok(()),
        Err(queue_error) => match state.object_store.delete(&segment.object_key) {
            Ok(()) => Ok(()),
            Err(delete_error) => Err(ApiError::service_unavailable(format!(
                "failed to queue or delete incomplete Git segment: {}; {}",
                queue_error.message, delete_error.message
            ))),
        },
    }
}

pub(crate) fn git_snapshot_from_ref(
    state: &AppState,
    repo_id: &str,
    repo: &FsPath,
    refname: &str,
) -> Result<SourceBlob, ApiError> {
    git_snapshot_from_refs(state, repo_id, repo, &[refname.to_string()])
}

fn git_snapshot_from_refs(
    state: &AppState,
    repo_id: &str,
    repo: &FsPath,
    refs: &[String],
) -> Result<SourceBlob, ApiError> {
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
    let mut snapshot =
        put_repo_object(state.object_store.as_ref(), repo_id, "git-bundles", &bytes)?;
    snapshot.git_oid = head_oid.trim().to_string();
    Ok(snapshot)
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
    pub(crate) path: String,
    pub(crate) mode: String,
    pub(crate) oid: String,
    pub(crate) size_bytes: usize,
}

pub(crate) fn validate_pushed_file_path(path: &str) -> Result<(), ApiError> {
    if path.is_empty() || path.starts_with('/') || path.contains('\\') {
        return Err(ApiError::bad_request(format!(
            "unsupported Git file path {path:?}"
        )));
    }
    if path.bytes().any(|byte| byte < 0x20 || byte == 0x7f) {
        return Err(ApiError::bad_request(format!(
            "unsupported Git file path {path:?}"
        )));
    }

    let scope_path = ScopePath::parse(format!("/{path}")).map_err(ApiError::bad_request)?;
    if scope_path.as_str() != format!("/{path}") {
        return Err(ApiError::bad_request(format!(
            "unsupported Git file path {path:?}"
        )));
    }
    if is_reserved_config_path(&scope_path) {
        return Err(ApiError::bad_request(format!(
            "Scope config path {path:?} is a local sidecar file and cannot be pushed"
        )));
    }

    Ok(())
}

pub(crate) fn run_git(repo: Option<&FsPath>, args: &[&str], action: &str) -> Result<(), ApiError> {
    let output = run_git_output(repo, args, action)?;
    if output.status.success() {
        Ok(())
    } else {
        Err(ApiError::service_unavailable(format!(
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
        return Err(ApiError::service_unavailable(format!(
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
    .map_err(|error| ApiError::service_unavailable(format!("failed {action}: {}", error.message())))
}

pub(crate) fn safe_repo_key(owner: &str, repo_name: &str) -> String {
    let repo_id = crate::domain::store::repo_id(owner, repo_name);
    let digest = Sha256::digest(repo_id.as_bytes());
    format!("repo-{digest:x}")
}
