use crate::domain::policy::ScopePath;
use crate::domain::store::{PendingImportFile, SourceBlob};
use crate::{
    config::{
        DEFAULT_GIT_BRANCH, MAX_PENDING_IMPORT_BLOB_BYTES, MAX_PENDING_IMPORT_FILES,
        MAX_PENDING_IMPORT_TOTAL_BYTES,
    },
    error::ApiError,
    object_store::put_repo_object,
    persistence::unix_now,
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
    let output = run_git_output(
        Some(staging_repo),
        &[
            "for-each-ref",
            "--format=%(refname)%00%(objectname)",
            "refs/heads",
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

pub(crate) fn git_tree_files(
    state: &AppState,
    repo_id: &str,
    staging_repo: &FsPath,
    head_oid: &str,
) -> Result<Vec<PendingImportFile>, ApiError> {
    let pending_files = git_tree_entries(staging_repo, head_oid)?;
    let contents = git_tree_blob_contents(staging_repo, &pending_files)?;
    let mut files = Vec::with_capacity(pending_files.len());
    let mut uploaded_blobs = Vec::with_capacity(pending_files.len());
    let blobs = match put_git_blob_contents(state, repo_id, &contents, &mut uploaded_blobs) {
        Ok(blobs) => blobs,
        Err(error) => {
            crate::state::best_effort_cleanup_rollback_source_blobs(state, &uploaded_blobs);
            return Err(error);
        }
    };
    for (pending, blob) in pending_files.into_iter().zip(blobs) {
        files.push(PendingImportFile {
            path: pending.path,
            mode: pending.mode,
            oid: pending.oid,
            blob,
        });
    }
    files.sort_by(|left, right| left.path.cmp(&right.path));
    Ok(files)
}

pub(super) fn git_tree_entries(
    staging_repo: &FsPath,
    head_oid: &str,
) -> Result<Vec<PendingGitTreeFile>, ApiError> {
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
        if mode != "100644" {
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
        pending_files.push(PendingGitTreeFile {
            path: path.to_string(),
            mode: mode.to_string(),
            oid: oid.to_string(),
            size_bytes: blob_size,
        });
    }

    Ok(pending_files)
}

pub(super) fn git_tree_blob_contents(
    staging_repo: &FsPath,
    pending_files: &[PendingGitTreeFile],
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
            Ok(())
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
    contents: &[Vec<u8>],
    uploaded_blobs: &mut Vec<SourceBlob>,
) -> Result<Vec<SourceBlob>, ApiError> {
    let store = state.object_store.as_ref();
    let mut blobs = Vec::with_capacity(contents.len());
    for chunk in contents.chunks(MAX_PARALLEL_BLOB_PUTS) {
        let results = thread::scope(|scope| {
            let handles = chunk
                .iter()
                .map(|content| {
                    scope.spawn(move || put_repo_object(store, repo_id, "blobs", content))
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
    pending_files: &[PendingGitTreeFile],
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

        std::str::from_utf8(&content).map_err(|_| {
            ApiError::bad_request(format!("blob {} must be valid UTF-8 text", pending.path))
        })?;
        contents.push(content);
    }

    if cursor != output.len() {
        return Err(ApiError::internal_message(
            "git cat-file batch output has trailing data",
        ));
    }

    Ok(contents)
}

pub(super) fn git_snapshot_from_repo(
    state: &AppState,
    repo_id: &str,
    repo: &FsPath,
) -> Result<SourceBlob, ApiError> {
    let bundle_path = repo.join(format!(
        "scope-snapshot-{}-{}.bundle",
        std::process::id(),
        unix_now()?
    ));
    let bundle = bundle_path.to_string_lossy().to_string();
    run_git(
        Some(repo),
        &["bundle", "create", &bundle, "--all"],
        "creating Git snapshot bundle",
    )?;
    let bytes = std::fs::read(&bundle_path).map_err(ApiError::internal)?;
    let _ = std::fs::remove_file(&bundle_path);
    put_repo_object(state.object_store.as_ref(), repo_id, "git-bundles", &bytes)
}

pub(super) struct PendingGitTreeFile {
    pub(super) path: String,
    mode: String,
    pub(super) oid: String,
    pub(super) size_bytes: usize,
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
    command
        .args(args)
        .output()
        .map_err(|error| ApiError::service_unavailable(format!("failed {action}: {error}")))
}

pub(crate) fn safe_repo_key(owner: &str, repo_name: &str) -> String {
    let repo_id = crate::domain::store::repo_id(owner, repo_name);
    let digest = Sha256::digest(repo_id.as_bytes());
    format!("repo-{digest:x}")
}
