use crate::{
    error::ApiError,
    git::{
        import::{run_git_output, run_git_output_bounded},
        request_refs::with_request_revision_store_repo,
    },
    http::{
        file_diffs::{
            MAX_RENDERED_TEXT_BYTES, binary_content_response, review_content_response_for_bytes,
        },
        requests::{repo_and_access, visible_request},
        responses::{
            CommitFileResponse, RequestFileDiffRequest, RequestRevisionCommitFilesResponse,
            ReviewFileContentResponse, ReviewFileDiffResponse, request_actor_summary_response,
        },
    },
    state::AppState,
};
use axum::{
    Json,
    extract::{Path, Query, State},
    http::HeaderMap,
};
use scope_api_contract::{
    GitOid, RequestDiscussionAnchor as RequestDiscussionAnchorRequest,
    RequestRevisionCommitResponse, RequestRevisionListResponse, RequestRevisionResponse,
};
use scope_domain::{
    policy::ScopePath,
    requests::{Request, RequestDiscussionAnchor, RequestRevision},
    store::{FileChangeKind, RepositoryAccess, StoredRepository},
};
use serde::Deserialize;
use std::path::Path as FsPath;

const MAX_LISTED_REQUEST_REVISIONS: usize = 50;
const MAX_LISTED_COMMITS_PER_REVISION: usize = 100;
const MAX_LISTED_REQUEST_COMMITS: usize = 100;
const MAX_IMPORTED_REQUEST_REVISIONS: usize = 5;
const MAX_IMPORTED_REQUEST_SNAPSHOT_BYTES: u64 = 64 * 1024 * 1024;
const MAX_REQUEST_COMMIT_METADATA_BYTES: usize = 64 * 1024;

#[derive(Deserialize)]
pub(crate) struct RequestRevisionListRequest {
    revision: Option<String>,
    commit: Option<String>,
}

pub(crate) async fn list_request_revisions(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((owner, repo_name, request_id)): Path<(String, String, String)>,
    Query(input): Query<RequestRevisionListRequest>,
) -> Result<Json<RequestRevisionListResponse>, ApiError> {
    let (repo, access, viewer_user_id) =
        repo_and_access(&state, &headers, &owner, &repo_name).await?;
    let request = visible_request(
        &state,
        &repo,
        access,
        viewer_user_id.as_deref(),
        &request_id,
    )
    .await?;
    if input.commit.is_some() && input.revision.is_none() {
        return Err(ApiError::bad_request(
            "selecting a request commit requires a revision",
        ));
    }
    let revision_window = state
        .metadata
        .requests()
        .request_revision_window(
            &request.id,
            input.revision.as_deref(),
            MAX_LISTED_REQUEST_REVISIONS as u64,
        )
        .await?;
    let selected_commit = input.commit.map(canonical_commit_oid).transpose()?;
    let revisions = revision_window.revisions;
    let users = state
        .metadata
        .requests()
        .users_by_ids(
            revisions
                .iter()
                .map(|revision| revision.actor_user_id.clone()),
        )
        .await?;
    let mut responses = vec![None; revisions.len()];
    let selected_revision_index = input.revision.as_deref().and_then(|revision_id| {
        revisions
            .iter()
            .position(|revision| revision.id == revision_id)
    });
    let mut processing_order = selected_revision_index.into_iter().collect::<Vec<_>>();
    processing_order.extend(
        (0..revisions.len())
            .rev()
            .filter(|index| Some(*index) != selected_revision_index),
    );
    let mut work_budget = RequestRevisionListWorkBudget::new();
    for index in processing_order {
        let revision = &revisions[index];
        let commit_limit = work_budget.claim_revision(revision.git_snapshot.size_bytes);
        let (commits, commits_truncated, commits_inspected) =
            if let Some(commit_limit) = commit_limit {
                let (commits, commits_truncated) = with_request_revision_store_repo(
                    &state,
                    &owner,
                    &repo_name,
                    &request,
                    revision,
                    |raw_repo| {
                        request_revision_commits(
                            raw_repo,
                            &repo,
                            access,
                            revision,
                            (input.revision.as_deref() == Some(revision.id.as_str()))
                                .then_some(selected_commit.as_deref())
                                .flatten(),
                            commit_limit,
                        )
                    },
                )?;
                let commits_inspected = commits.inspected;
                (commits.visible, commits_truncated, commits_inspected)
            } else {
                (Vec::new(), true, 0)
            };
        work_budget.record_inspected(commits_inspected);
        responses[index] = Some(RequestRevisionResponse {
            id: revision.id.clone(),
            position: revision.position,
            actor: request_actor_summary_response(&revision.actor_user_id, &users)?,
            old_head_oid: revision.old_head_oid.clone(),
            new_head_oid: revision.new_head_oid.clone(),
            commits,
            commits_truncated,
            created_at_unix: revision.created_at_unix,
        });
    }
    let revisions = responses
        .into_iter()
        .map(|response| response.expect("every request revision is processed once"))
        .collect();
    Ok(Json(RequestRevisionListResponse {
        revisions,
        has_earlier_revisions: revision_window.has_earlier_revisions,
    }))
}

struct RequestRevisionListWorkBudget {
    remaining_commits: usize,
    remaining_revisions: usize,
    remaining_snapshot_bytes: u64,
}

impl RequestRevisionListWorkBudget {
    fn new() -> Self {
        Self {
            remaining_commits: MAX_LISTED_REQUEST_COMMITS,
            remaining_revisions: MAX_IMPORTED_REQUEST_REVISIONS,
            remaining_snapshot_bytes: MAX_IMPORTED_REQUEST_SNAPSHOT_BYTES,
        }
    }

    fn claim_revision(&mut self, snapshot_bytes: u64) -> Option<usize> {
        let commit_limit = self.remaining_commits.min(MAX_LISTED_COMMITS_PER_REVISION);
        if commit_limit == 0
            || self.remaining_revisions == 0
            || snapshot_bytes > self.remaining_snapshot_bytes
        {
            return None;
        }
        self.remaining_revisions -= 1;
        self.remaining_snapshot_bytes -= snapshot_bytes;
        Some(commit_limit)
    }

    fn record_inspected(&mut self, commits: usize) {
        self.remaining_commits = self.remaining_commits.saturating_sub(commits);
    }
}

pub(crate) async fn get_request_revision_commit(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((owner, repo_name, request_id, revision_id, commit_oid)): Path<(
        String,
        String,
        String,
        String,
        String,
    )>,
) -> Result<Json<RequestRevisionCommitFilesResponse>, ApiError> {
    let (repo, access, viewer_user_id) =
        repo_and_access(&state, &headers, &owner, &repo_name).await?;
    let request = visible_request(
        &state,
        &repo,
        access,
        viewer_user_id.as_deref(),
        &request_id,
    )
    .await?;
    let revision = state
        .metadata
        .requests()
        .request_revision(&request.id, &revision_id)
        .await?
        .ok_or_else(|| ApiError::not_found("request revision not found"))?;
    let commit_oid = canonical_commit_oid(commit_oid)?;
    let (commit, files) = with_request_revision_store_repo(
        &state,
        &owner,
        &repo_name,
        &request,
        &revision,
        |raw_repo| request_revision_commit_files(raw_repo, &repo, access, &revision, &commit_oid),
    )?;
    Ok(Json(RequestRevisionCommitFilesResponse {
        revision_id: revision.id,
        commit,
        files,
    }))
}

pub(crate) async fn get_request_revision_commit_file_diff(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((owner, repo_name, request_id, revision_id, commit_oid)): Path<(
        String,
        String,
        String,
        String,
        String,
    )>,
    Query(input): Query<RequestFileDiffRequest>,
) -> Result<Json<ReviewFileDiffResponse>, ApiError> {
    let (repo, access, viewer_user_id) =
        repo_and_access(&state, &headers, &owner, &repo_name).await?;
    let request = visible_request(
        &state,
        &repo,
        access,
        viewer_user_id.as_deref(),
        &request_id,
    )
    .await?;
    let revision = state
        .metadata
        .requests()
        .request_revision(&request.id, &revision_id)
        .await?
        .ok_or_else(|| ApiError::not_found("request revision not found"))?;
    let commit_oid = canonical_commit_oid(commit_oid)?;
    let path = normalized_path(&input.path)?;
    let (file, old_content, new_content) = with_request_revision_store_repo(
        &state,
        &owner,
        &repo_name,
        &request,
        &revision,
        |raw_repo| {
            let (_, files) =
                request_revision_commit_files(raw_repo, &repo, access, &revision, &commit_oid)?;
            let file = files
                .into_iter()
                .find(|file| file.path == path)
                .ok_or_else(|| ApiError::not_found("request revision file not found"))?;
            let old_content = file
                .old_oid
                .as_deref()
                .map(|oid| git_blob_content(raw_repo, oid))
                .transpose()?;
            let new_content = file
                .new_oid
                .as_deref()
                .map(|oid| git_blob_content(raw_repo, oid))
                .transpose()?;
            Ok((file, old_content, new_content))
        },
    )?;
    Ok(Json(ReviewFileDiffResponse {
        path,
        kind: file.kind,
        old_mode: file.old_mode,
        new_mode: file.new_mode,
        old_content,
        new_content,
    }))
}

pub(crate) async fn validate_request_discussion_anchor(
    state: &AppState,
    owner: &str,
    repo_name: &str,
    repo: &StoredRepository,
    access: RepositoryAccess,
    request: &Request,
    anchor: RequestDiscussionAnchorRequest,
) -> Result<RequestDiscussionAnchor, ApiError> {
    if anchor.path.is_some() && anchor.commit_oid.is_none() {
        return Err(ApiError::bad_request(
            "request discussion path requires a commit",
        ));
    }
    let revision = state
        .metadata
        .requests()
        .request_revision(&request.id, &anchor.revision_id)
        .await?
        .ok_or_else(|| ApiError::not_found("request revision not found"))?;
    let path = anchor
        .path
        .map(|path| normalized_scope_path(&path))
        .transpose()?;
    let commit_oid = anchor.commit_oid.map(canonical_commit_oid).transpose()?;
    if let Some(commit_oid) = commit_oid.as_deref() {
        let (_, files) = with_request_revision_store_repo(
            state,
            owner,
            repo_name,
            request,
            &revision,
            |raw_repo| request_revision_commit_files(raw_repo, repo, access, &revision, commit_oid),
        )?;
        if let Some(path) = path.as_ref()
            && !files
                .iter()
                .any(|file| file.path == path.as_str().trim_start_matches('/'))
        {
            return Err(ApiError::bad_request(
                "request discussion path is not changed by the selected commit",
            ));
        }
    }
    Ok(RequestDiscussionAnchor {
        revision_id: revision.id,
        commit_oid,
        path,
    })
}

fn request_revision_commits(
    raw_repo: &FsPath,
    repo: &StoredRepository,
    access: RepositoryAccess,
    revision: &RequestRevision,
    selected_commit: Option<&str>,
    limit: usize,
) -> Result<(InspectedRequestCommits, bool), ApiError> {
    let mut commit_oids = request_revision_commit_oids(raw_repo, revision, limit)?;
    let mut commits_truncated = commit_oids.len() > limit;
    if commits_truncated {
        commit_oids.drain(..commit_oids.len() - limit);
    }
    if let Some(selected_commit) = selected_commit
        && !commit_oids.iter().any(|commit| commit == selected_commit)
        && commit_belongs_to_revision(raw_repo, revision, selected_commit)?
    {
        if commit_oids.len() == limit {
            commit_oids.remove(0);
            commits_truncated = true;
        }
        commit_oids.insert(0, selected_commit.to_string());
    }
    let inspected = commit_oids.len();
    let visible = commit_oids
        .into_iter()
        .map(|commit_oid| request_commit_summary(raw_repo, repo, access, &commit_oid))
        .collect::<Result<Vec<_>, ApiError>>()?
        .into_iter()
        .flatten()
        .collect::<Vec<_>>();
    Ok((
        InspectedRequestCommits { visible, inspected },
        commits_truncated,
    ))
}

struct InspectedRequestCommits {
    visible: Vec<RequestRevisionCommitResponse>,
    inspected: usize,
}

fn request_revision_commit_files(
    raw_repo: &FsPath,
    repo: &StoredRepository,
    access: RepositoryAccess,
    revision: &RequestRevision,
    commit_oid: &str,
) -> Result<(RequestRevisionCommitResponse, Vec<CommitFileResponse>), ApiError> {
    if !commit_belongs_to_revision(raw_repo, revision, commit_oid)? {
        return Err(ApiError::not_found("request revision commit not found"));
    }
    let commit = request_commit_summary(raw_repo, repo, access, commit_oid)?
        .ok_or_else(|| ApiError::not_found("request revision commit not found"))?;
    let parent = commit
        .parent_oids
        .first()
        .ok_or_else(|| ApiError::conflict("request revision commit must have a parent"))?;
    let files = request_changes_from_repo(raw_repo, repo, access, parent, commit_oid, None)?;
    Ok((commit, files))
}

fn request_revision_commit_oids(
    raw_repo: &FsPath,
    revision: &RequestRevision,
    limit: usize,
) -> Result<Vec<String>, ApiError> {
    let exclude_old_head = format!("^{}", revision.old_head_oid);
    let max_count = format!("--max-count={}", limit.saturating_add(1));
    let max_output_bytes = limit.saturating_add(1).saturating_mul(41);
    let output = run_git_output_bounded(
        Some(raw_repo),
        &[
            "rev-list",
            &max_count,
            "--reverse",
            "--topo-order",
            &revision.new_head_oid,
            &exclude_old_head,
        ],
        "reading request revision commits",
        max_output_bytes,
    )?;
    if !output.status.success() {
        return Err(ApiError::infrastructure_unavailable(format!(
            "reading request revision commits: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        )));
    }
    Ok(String::from_utf8(output.stdout)
        .map_err(ApiError::bad_request)?
        .lines()
        .filter(|line| !line.trim().is_empty())
        .map(ToString::to_string)
        .collect())
}

fn commit_belongs_to_revision(
    raw_repo: &FsPath,
    revision: &RequestRevision,
    commit_oid: &str,
) -> Result<bool, ApiError> {
    if !git_commit_exists(raw_repo, commit_oid)? {
        return Ok(false);
    }
    if !git_is_ancestor(raw_repo, commit_oid, &revision.new_head_oid)? {
        return Ok(false);
    }
    Ok(!git_is_ancestor(
        raw_repo,
        commit_oid,
        &revision.old_head_oid,
    )?)
}

fn git_commit_exists(raw_repo: &FsPath, commit_oid: &str) -> Result<bool, ApiError> {
    let commit_object = format!("{commit_oid}^{{commit}}");
    let output = run_git_output(
        Some(raw_repo),
        &["cat-file", "-e", &commit_object],
        "validating request revision commit",
    )?;
    match output.status.code() {
        Some(0) => Ok(true),
        Some(1 | 128) => Ok(false),
        _ => Err(ApiError::infrastructure_unavailable(format!(
            "validating request revision commit: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ))),
    }
}

fn git_is_ancestor(raw_repo: &FsPath, ancestor: &str, descendant: &str) -> Result<bool, ApiError> {
    let output = run_git_output(
        Some(raw_repo),
        &["merge-base", "--is-ancestor", ancestor, descendant],
        "validating request revision commit",
    )?;
    match output.status.code() {
        Some(0) => Ok(true),
        Some(1) => Ok(false),
        _ => Err(ApiError::infrastructure_unavailable(format!(
            "validating request revision commit: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ))),
    }
}

fn request_commit_summary(
    raw_repo: &FsPath,
    repo: &StoredRepository,
    access: RepositoryAccess,
    commit_oid: &str,
) -> Result<Option<RequestRevisionCommitResponse>, ApiError> {
    let Some(output) = omit_oversized_commit_metadata(run_git_output_bounded(
        Some(raw_repo),
        &[
            "show",
            "-s",
            "--format=%P%x00%an <%ae>%x00%at%x00%B",
            commit_oid,
        ],
        "reading request commit metadata",
        MAX_REQUEST_COMMIT_METADATA_BYTES,
    ))?
    else {
        return Ok(None);
    };
    if !output.status.success() {
        return Err(ApiError::infrastructure_unavailable(format!(
            "reading request commit metadata: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        )));
    }
    let metadata = request_commit_metadata(&output.stdout)?;
    // Request-ref validation requires every revision head to descend from its recorded base,
    // so commits introduced by a revision cannot be parentless roots.
    let parent = metadata
        .parent_oids
        .first()
        .ok_or_else(|| ApiError::conflict("request revision commit must have a parent"))?;
    let changes = request_changes_from_repo_with_visibility(
        raw_repo, repo, access, parent, commit_oid, None,
    )?;
    if !access.can_read_private_files && changes.hidden {
        return Ok(None);
    }
    Ok(Some(RequestRevisionCommitResponse {
        oid: commit_oid.to_string(),
        parent_oids: metadata.parent_oids,
        author: metadata.author,
        authored_at_unix: metadata.authored_at_unix,
        message: metadata.message,
        change_count: changes.files.len(),
    }))
}

fn omit_oversized_commit_metadata(
    output: Result<std::process::Output, ApiError>,
) -> Result<Option<std::process::Output>, ApiError> {
    match output {
        Ok(output) => Ok(Some(output)),
        Err(error) if error.status() == axum::http::StatusCode::PAYLOAD_TOO_LARGE => Ok(None),
        Err(error) => Err(error),
    }
}

struct RequestCommitMetadata {
    parent_oids: Vec<String>,
    author: Option<String>,
    authored_at_unix: u64,
    message: String,
}

fn request_commit_metadata(output: &[u8]) -> Result<RequestCommitMetadata, ApiError> {
    let mut fields = output.splitn(4, |byte| *byte == 0);
    let parent_oids = fields
        .next()
        .unwrap_or_default()
        .split(|byte| byte.is_ascii_whitespace())
        .filter(|value| !value.is_empty())
        .map(|value| String::from_utf8_lossy(value).into_owned())
        .collect::<Vec<_>>();
    let author = fields
        .next()
        .map(String::from_utf8_lossy)
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty());
    let authored_at_unix = std::str::from_utf8(fields.next().unwrap_or_default())
        .map_err(ApiError::bad_request)?
        .trim()
        .parse::<u64>()
        .map_err(ApiError::bad_request)?;
    let message = String::from_utf8_lossy(fields.next().unwrap_or_default())
        .trim()
        .to_string();
    Ok(RequestCommitMetadata {
        parent_oids,
        author,
        authored_at_unix,
        message,
    })
}

fn request_changes_from_repo(
    raw_repo: &FsPath,
    repo: &StoredRepository,
    access: RepositoryAccess,
    old_head_oid: &str,
    new_head_oid: &str,
    path: Option<&str>,
) -> Result<Vec<CommitFileResponse>, ApiError> {
    Ok(request_changes_from_repo_with_visibility(
        raw_repo,
        repo,
        access,
        old_head_oid,
        new_head_oid,
        path,
    )?
    .files)
}

struct VisibleRequestChanges {
    files: Vec<CommitFileResponse>,
    hidden: bool,
}

fn request_changes_from_repo_with_visibility(
    raw_repo: &FsPath,
    repo: &StoredRepository,
    access: RepositoryAccess,
    old_head_oid: &str,
    new_head_oid: &str,
    path: Option<&str>,
) -> Result<VisibleRequestChanges, ApiError> {
    let mut args = vec![
        "--literal-pathspecs",
        "diff",
        "--raw",
        "-z",
        "--no-renames",
        "--abbrev=64",
        old_head_oid,
        new_head_oid,
        "--",
    ];
    if let Some(path) = path {
        args.push(path);
    }
    let output = run_git_output(Some(raw_repo), &args, "reading request changes")?;
    if !output.status.success() {
        return Err(ApiError::infrastructure_unavailable(format!(
            "reading request changes: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        )));
    }

    let mut fields = output.stdout.split(|byte| *byte == 0);
    let mut files = Vec::new();
    let mut hidden = false;
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
            hidden = true;
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
            kind: kind.into(),
            old_mode: git_mode(columns[0].trim_start_matches(':')),
            new_mode: git_mode(columns[1]),
            old_oid,
            new_oid,
            visibility: repo.policy.effective_visibility(&scope_path).into(),
        });
    }
    files.sort_by(|left, right| left.path.cmp(&right.path));
    Ok(VisibleRequestChanges { files, hidden })
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
        return Err(ApiError::infrastructure_unavailable(format!(
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
        Err(ApiError::infrastructure_unavailable(format!(
            "reading request file: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        )))
    }
}

fn normalized_path(path: &str) -> Result<String, ApiError> {
    let scope_path = normalized_scope_path(path)?;
    Ok(scope_path.as_str().trim_start_matches('/').to_string())
}

fn canonical_commit_oid(oid: String) -> Result<String, ApiError> {
    GitOid::try_from(oid)
        .map(String::from)
        .map_err(ApiError::bad_request)
}

fn normalized_scope_path(path: &str) -> Result<ScopePath, ApiError> {
    let scope_path = ScopePath::parse(format!("/{}", path.trim_start_matches('/')))
        .map_err(ApiError::bad_request)?;
    if scope_path == ScopePath::root() {
        return Err(ApiError::bad_request("file path is required"));
    }
    Ok(scope_path)
}

#[cfg(test)]
mod tests {
    use super::{
        ApiError, MAX_IMPORTED_REQUEST_REVISIONS, MAX_IMPORTED_REQUEST_SNAPSHOT_BYTES,
        RequestRevisionListWorkBudget, omit_oversized_commit_metadata, request_commit_metadata,
    };

    #[test]
    fn commit_metadata_decodes_non_utf8_display_fields_lossily() {
        let metadata = request_commit_metadata(
            b"0123456789012345678901234567890123456789\0Ada \xff <ada@example.com>\x001700000000\0Fix \xfe metadata\n",
        )
        .unwrap();

        assert_eq!(metadata.parent_oids.len(), 1);
        assert_eq!(metadata.author.as_deref(), Some("Ada � <ada@example.com>"));
        assert_eq!(metadata.authored_at_unix, 1_700_000_000);
        assert_eq!(metadata.message, "Fix � metadata");
    }

    #[test]
    fn revision_listing_budget_caps_snapshot_count_bytes_and_commit_work() {
        let mut count_budget = RequestRevisionListWorkBudget::new();
        for _ in 0..MAX_IMPORTED_REQUEST_REVISIONS {
            assert!(count_budget.claim_revision(1).is_some());
            count_budget.record_inspected(1);
        }
        assert_eq!(count_budget.claim_revision(1), None);

        let mut byte_budget = RequestRevisionListWorkBudget::new();
        assert!(
            byte_budget
                .claim_revision(MAX_IMPORTED_REQUEST_SNAPSHOT_BYTES)
                .is_some()
        );
        assert_eq!(byte_budget.claim_revision(1), None);

        let mut commit_budget = RequestRevisionListWorkBudget::new();
        commit_budget.record_inspected(usize::MAX);
        assert_eq!(commit_budget.claim_revision(1), None);
    }

    #[test]
    fn oversized_commit_metadata_is_omitted_without_failing_the_listing() {
        let result = omit_oversized_commit_metadata(Err(ApiError::payload_too_large(
            "commit metadata too large",
        )))
        .unwrap();
        assert!(result.is_none());
    }
}
