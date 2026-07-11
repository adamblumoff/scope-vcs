use crate::domain::store::{SourceBlob, is_supported_git_file_mode};
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
use sha2::{Digest, Sha256};
use std::{
    io::Write,
    path::Path as FsPath,
    process::{Command, Stdio},
    thread,
};

const MAX_PARALLEL_BLOB_PUTS: usize = 8;

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

pub(crate) fn validate_pushed_tree(staging_repo: &FsPath, head_oid: &str) -> Result<(), ApiError> {
    git_tree_entries(staging_repo, head_oid).map(|_| ())
}

pub(super) fn git_tree_blob_contents(
    staging_repo: &FsPath,
    pending_files: &[GitTreeFile],
) -> Result<Vec<Vec<u8>>, ApiError> {
    if pending_files.is_empty() {
        return Ok(Vec::new());
    }

    let mut child = Command::new("git")
        .current_dir(staging_repo)
        .args(["cat-file", "--batch"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(ApiError::internal)?;
    let mut stdin = child
        .stdin
        .take()
        .ok_or_else(|| ApiError::internal_message("opening git cat-file stdin failed"))?;

    let (output, write_result) = thread::scope(|scope| {
        let writer = scope.spawn(move || {
            for pending in pending_files {
                writeln!(stdin, "{}", pending.oid).map_err(ApiError::internal)?;
            }
            Ok::<(), ApiError>(())
        });
        let output = child.wait_with_output().map_err(ApiError::internal);
        let write_result = writer
            .join()
            .map_err(|_| ApiError::internal_message("git cat-file input writer panicked"));
        (output, write_result)
    });
    let output = output?;
    if !output.status.success() {
        return Err(ApiError::service_unavailable(format!(
            "reading pushed blobs: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        )));
    }
    match write_result {
        Ok(result) => result?,
        Err(error) => return Err(error),
    }

    parse_git_cat_file_batch(&output.stdout, pending_files)
}

pub(super) fn put_git_blob_contents(
    state: &AppState,
    repo_id: &str,
    pending_files: &[GitTreeFile],
    contents: &[Vec<u8>],
    uploaded_blobs: &mut Vec<SourceBlob>,
) -> Result<Vec<SourceBlob>, ApiError> {
    if pending_files.len() != contents.len() {
        return Err(ApiError::internal_message(
            "Git tree entry count did not match blob content count",
        ));
    }
    let store = state.object_store.as_ref();
    let blob_put_parallelism = state
        .runtime_budgets
        .object_store_concurrency()
        .clamp(1, MAX_PARALLEL_BLOB_PUTS);
    let mut blobs = Vec::with_capacity(contents.len());
    for chunk in pending_files
        .iter()
        .zip(contents)
        .collect::<Vec<_>>()
        .chunks(blob_put_parallelism)
    {
        let results = thread::scope(|scope| {
            let handles = chunk
                .iter()
                .map(|(pending, content)| {
                    scope.spawn(move || {
                        let mut blob = put_repo_object(store, repo_id, "blobs", content)?;
                        blob.git_file_mode = pending.mode.clone();
                        Ok(blob)
                    })
                })
                .collect::<Vec<_>>();
            handles
                .into_iter()
                .map(|handle| handle.join())
                .collect::<Vec<_>>()
        });

        let mut first_error = None;
        for result in results {
            match result {
                Ok(Ok(blob)) => {
                    uploaded_blobs.push(blob.clone());
                    blobs.push(blob);
                }
                Ok(Err(error)) => {
                    first_error.get_or_insert(error);
                }
                Err(_) => {
                    first_error.get_or_insert_with(|| {
                        ApiError::internal_message("blob upload worker panicked")
                    });
                }
            }
        }
        if let Some(error) = first_error {
            return Err(error);
        }
    }

    Ok(blobs)
}

fn parse_git_cat_file_batch(
    output: &[u8],
    pending_files: &[GitTreeFile],
) -> Result<Vec<Vec<u8>>, ApiError> {
    let mut cursor = 0usize;
    let mut contents = Vec::with_capacity(pending_files.len());

    for pending in pending_files {
        let header_end = output[cursor..]
            .iter()
            .position(|byte| *byte == b'\n')
            .map(|offset| cursor + offset)
            .ok_or_else(|| ApiError::internal_message("git cat-file batch header missing"))?;
        let header = std::str::from_utf8(&output[cursor..header_end])
            .map_err(|_| ApiError::internal_message("git cat-file batch header is invalid"))?;
        cursor = header_end + 1;

        let mut fields = header.split_whitespace();
        let oid = fields
            .next()
            .ok_or_else(|| ApiError::internal_message("git cat-file batch header missing oid"))?;
        let kind = fields
            .next()
            .ok_or_else(|| ApiError::internal_message("git cat-file batch header missing kind"))?;
        let size = fields
            .next()
            .ok_or_else(|| ApiError::internal_message("git cat-file batch header missing size"))?
            .parse::<usize>()
            .map_err(|_| ApiError::internal_message("git cat-file batch size is invalid"))?;
        if oid != pending.oid || kind != "blob" || size != pending.size_bytes {
            return Err(ApiError::internal_message(
                "git cat-file batch output mismatch",
            ));
        }

        let content_end = cursor
            .checked_add(size)
            .ok_or_else(|| ApiError::internal_message("git cat-file batch output is too large"))?;
        if content_end >= output.len() {
            return Err(ApiError::internal_message(
                "git cat-file batch content is truncated",
            ));
        }
        let content = output[cursor..content_end].to_vec();
        cursor = content_end;
        if output.get(cursor) != Some(&b'\n') {
            return Err(ApiError::internal_message(
                "git cat-file batch content delimiter missing",
            ));
        }
        cursor += 1;

        contents.push(content);
    }

    if cursor != output.len() {
        return Err(ApiError::internal_message(
            "git cat-file batch output has trailing data",
        ));
    }

    Ok(contents)
}

pub(crate) fn git_snapshot_from_repo(
    state: &AppState,
    repo_id: &str,
    repo: &FsPath,
) -> Result<SourceBlob, ApiError> {
    git_snapshot_from_refs(
        state,
        repo_id,
        repo,
        &[format!("refs/heads/{DEFAULT_GIT_BRANCH}")],
    )
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
    let bundle_path = repo.join(format!("scope-snapshot-{}.bundle", random_bundle_id()?));
    let bundle = bundle_path.to_string_lossy().to_string();
    let mut args = vec!["bundle", "create", bundle.as_str()];
    args.extend(refs.iter().map(String::as_str));
    run_git(Some(repo), &args, "creating Git snapshot bundle")?;
    let bytes = std::fs::read(&bundle_path).map_err(ApiError::internal)?;
    let _ = std::fs::remove_file(&bundle_path);
    Ok(put_repo_object(
        state.object_store.as_ref(),
        repo_id,
        "git-bundles",
        &bytes,
    )?)
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
