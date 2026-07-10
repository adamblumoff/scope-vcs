use crate::{
    domain::{
        policy::ScopePath,
        requests::Request,
        store::{FileChangeKind, RepositoryAccess, StoredRepository},
    },
    error::ApiError,
    git::{import::run_git_output, request_refs::with_request_ref_store_repo},
    http::{
        file_diffs::{
            MAX_RENDERED_TEXT_BYTES, binary_content_response, review_content_response_for_bytes,
        },
        requests::{repo_and_access, visible_request},
        responses::{
            CommitFileResponse, RequestChangesResponse, RequestFileDiffRequest,
            ReviewFileContentResponse, ReviewFileDiffResponse,
        },
    },
    state::AppState,
};
use axum::{
    Json,
    extract::{Path, Query, State},
    http::HeaderMap,
};
use std::path::Path as FsPath;

pub(crate) async fn get_request_changes(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((owner, repo_name, request_id)): Path<(String, String, String)>,
) -> Result<Json<RequestChangesResponse>, ApiError> {
    let (repo, access, _) = repo_and_access(&state, &headers, &owner, &repo_name).await?;
    let request = visible_request(&state, &repo, access, &request_id).await?;
    let files = request_changes(&state, &owner, &repo_name, &repo, access, &request)?;
    Ok(Json(RequestChangesResponse { files }))
}

pub(crate) async fn get_request_file_diff(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((owner, repo_name, request_id)): Path<(String, String, String)>,
    Query(input): Query<RequestFileDiffRequest>,
) -> Result<Json<ReviewFileDiffResponse>, ApiError> {
    let (repo, access, _) = repo_and_access(&state, &headers, &owner, &repo_name).await?;
    let request = visible_request(&state, &repo, access, &request_id).await?;
    let path = normalized_path(&input.path)?;
    if request.git_snapshot.is_none() {
        return Err(ApiError::not_found("request has no uploaded changes"));
    }
    let (file, old_content, new_content) =
        with_request_ref_store_repo(&state, &owner, &repo_name, &request, |raw_repo| {
            let file =
                request_changes_from_repo(raw_repo, &repo, access, &request, Some(path.as_str()))?
                    .into_iter()
                    .next()
                    .ok_or_else(|| ApiError::not_found("request file not found"))?;
            let old_content = match file.old_oid.as_deref() {
                Some(oid) => Some(git_blob_content(raw_repo, oid)?),
                None => None,
            };
            let new_content = match file.new_oid.as_deref() {
                Some(oid) => Some(git_blob_content(raw_repo, oid)?),
                None => None,
            };
            Ok((file, old_content, new_content))
        })?;

    Ok(Json(ReviewFileDiffResponse {
        path,
        kind: file.kind,
        old_mode: file.old_mode,
        new_mode: file.new_mode,
        old_content,
        new_content,
    }))
}

fn request_changes(
    state: &AppState,
    owner: &str,
    repo_name: &str,
    repo: &StoredRepository,
    access: RepositoryAccess,
    request: &Request,
) -> Result<Vec<CommitFileResponse>, ApiError> {
    if request.git_snapshot.is_none() {
        return Ok(Vec::new());
    }
    with_request_ref_store_repo(state, owner, repo_name, request, |raw_repo| {
        request_changes_from_repo(raw_repo, repo, access, request, None)
    })
}

fn request_changes_from_repo(
    raw_repo: &FsPath,
    repo: &StoredRepository,
    access: RepositoryAccess,
    request: &Request,
    path: Option<&str>,
) -> Result<Vec<CommitFileResponse>, ApiError> {
    let mut args = vec![
        "--literal-pathspecs",
        "diff",
        "--raw",
        "-z",
        "--no-renames",
        "--abbrev=64",
        &request.base_main_oid,
        &request.head_oid,
        "--",
    ];
    if let Some(path) = path {
        args.push(path);
    }
    let output = run_git_output(Some(raw_repo), &args, "reading request changes")?;
    if !output.status.success() {
        return Err(ApiError::service_unavailable(format!(
            "reading request changes: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        )));
    }

    let mut fields = output.stdout.split(|byte| *byte == 0);
    let mut files = Vec::new();
    while let Some(header) = fields.next() {
        if header.is_empty() {
            continue;
        }
        let header = std::str::from_utf8(header).map_err(ApiError::bad_request)?;
        let columns = header.split_ascii_whitespace().collect::<Vec<_>>();
        if columns.len() != 5 || !columns[0].starts_with(':') {
            return Err(ApiError::internal_message(format!(
                "invalid request diff header {header}"
            )));
        }
        let status = columns[4].as_bytes();
        let path = fields
            .next()
            .ok_or_else(|| ApiError::internal_message("request diff is missing a path"))?;
        let path = String::from_utf8(path.to_vec()).map_err(ApiError::bad_request)?;
        let scope_path = ScopePath::parse(format!("/{path}")).map_err(ApiError::bad_request)?;
        if !repo
            .policy
            .can_read(&scope_path, access.can_read_private_files)
        {
            continue;
        }
        let kind = match status[0] {
            b'A' => FileChangeKind::Added,
            b'M' | b'T' => FileChangeKind::Modified,
            b'D' => FileChangeKind::Deleted,
            _ => {
                return Err(ApiError::internal_message(format!(
                    "unsupported request diff status {}",
                    String::from_utf8_lossy(status)
                )));
            }
        };
        let old_oid = (kind != FileChangeKind::Added).then(|| columns[2].to_string());
        let new_oid = (kind != FileChangeKind::Deleted).then(|| columns[3].to_string());
        files.push(CommitFileResponse {
            path,
            kind,
            old_mode: git_mode(columns[0].trim_start_matches(':')),
            new_mode: git_mode(columns[1]),
            old_oid,
            new_oid,
            visibility: repo.policy.effective_visibility(&scope_path),
        });
    }
    files.sort_by(|left, right| left.path.cmp(&right.path));
    Ok(files)
}

fn git_mode(mode: &str) -> Option<String> {
    (mode != "000000").then(|| mode.to_string())
}

fn git_blob_content(repo: &FsPath, oid: &str) -> Result<ReviewFileContentResponse, ApiError> {
    let size_output = run_git_output(
        Some(repo),
        &["cat-file", "-s", oid],
        "reading request file size",
    )?;
    if !size_output.status.success() {
        return Err(ApiError::service_unavailable(format!(
            "reading request file size: {}",
            String::from_utf8_lossy(&size_output.stderr).trim()
        )));
    }
    let size = std::str::from_utf8(&size_output.stdout)
        .map_err(ApiError::bad_request)?
        .trim()
        .parse::<u64>()
        .map_err(ApiError::bad_request)?;
    if size > MAX_RENDERED_TEXT_BYTES as u64 {
        return Ok(binary_content_response(oid, size));
    }

    let output = run_git_output(
        Some(repo),
        &["cat-file", "blob", oid],
        "reading request file",
    )?;
    if output.status.success() {
        Ok(review_content_response_for_bytes(oid, &output.stdout))
    } else {
        Err(ApiError::service_unavailable(format!(
            "reading request file: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        )))
    }
}

fn normalized_path(path: &str) -> Result<String, ApiError> {
    let scope_path = ScopePath::parse(format!("/{path}")).map_err(ApiError::bad_request)?;
    if scope_path == ScopePath::root() {
        return Err(ApiError::bad_request("file path is required"));
    }
    Ok(scope_path.as_str().trim_start_matches('/').to_string())
}
