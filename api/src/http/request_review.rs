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
            RequestFileDiffRequest, ReviewFileContentResponse, ReviewFileDiffResponse,
            request_actor_summary_response,
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
    CommitFileResponse, GitOid, RequestRevisionCommitResponse, RequestRevisionInspectionState,
    RequestRevisionListResponse, RequestRevisionResponse,
};
use scope_domain::{
    history::FileChangeKind,
    policy::{Policy, ScopePath},
    repository::Repository,
    repository::access::RepositoryAccess,
    requests::{Request, RequestRevision, select_request_review_revision},
};
use serde::Deserialize;
use std::{path::Path as FsPath, sync::Arc};

mod inspection;

pub(crate) use inspection::RequestRevisionCommitVisibility;
use inspection::{
    inspect_request_commit, inspect_request_commits_identity_only, request_revision_commit_files,
};

const MAX_LISTED_REQUEST_REVISIONS: usize = 50;
const MAX_LISTED_COMMITS_PER_REVISION: usize = 100;
const MAX_LISTED_REQUEST_COMMITS: usize = 100;
const MAX_LISTED_REQUEST_FILES: usize = 10_000;
const MAX_IMPORTED_REQUEST_REVISIONS: usize = 5;
const MAX_IMPORTED_REQUEST_SNAPSHOT_BYTES: u64 = 64 * 1024 * 1024;

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
    let repo = Arc::new(repo);
    let request = visible_request(
        &state,
        &repo.record.id,
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
    let review_revision_id = select_request_review_revision(&revisions, input.revision.as_deref())?
        .map(|revision| revision.id.clone());
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
    let selected_revision_index = review_revision_id.as_deref().and_then(|revision_id| {
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
        let (commits, inspection) = if let Some(commit_limit) = commit_limit {
            let repo_for_inspection = repo.clone();
            let selected_commit = (input.revision.as_deref() == Some(revision.id.as_str()))
                .then(|| selected_commit.clone())
                .flatten();
            let file_limit = work_budget.remaining_files;
            let inspected = with_request_revision_store_repo(
                &state,
                &repo.incarnation(),
                &request,
                revision,
                move |raw_repo, revision| {
                    request_revision_commits(
                        raw_repo,
                        &repo_for_inspection,
                        access,
                        revision,
                        selected_commit.as_deref(),
                        commit_limit,
                        file_limit,
                    )
                },
            )
            .await?;
            work_budget.record_inspected(inspected.inspected, inspected.files_listed);
            (inspected.visible, inspected.inspection)
        } else {
            (Vec::new(), RequestRevisionInspectionState::Unavailable)
        };
        let (old_head_oid, new_head_oid) = if access.can_read_private_files {
            (
                Some(revision.old_head_oid.clone()),
                Some(revision.new_head_oid.clone()),
            )
        } else {
            (
                None,
                commits
                    .iter()
                    .any(|commit| commit.oid == revision.new_head_oid)
                    .then(|| revision.new_head_oid.clone()),
            )
        };
        responses[index] = Some(RequestRevisionResponse {
            id: revision.id.clone(),
            position: revision.position,
            actor: request_actor_summary_response(&revision.actor_user_id, &users)?,
            old_head_oid,
            new_head_oid,
            commits,
            inspection,
            created_at_unix: revision.created_at_unix,
        });
    }
    let revisions = responses
        .into_iter()
        .map(|response| response.expect("every request revision is processed once"))
        .collect();
    Ok(Json(RequestRevisionListResponse {
        review_revision_id,
        revisions,
        has_earlier_revisions: revision_window.has_earlier_revisions,
    }))
}

struct RequestRevisionListWorkBudget {
    remaining_commits: usize,
    remaining_files: usize,
    remaining_revisions: usize,
    remaining_snapshot_bytes: u64,
}

impl RequestRevisionListWorkBudget {
    fn new() -> Self {
        Self {
            remaining_commits: MAX_LISTED_REQUEST_COMMITS,
            remaining_files: MAX_LISTED_REQUEST_FILES,
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

    fn record_inspected(&mut self, commits: usize, files: usize) {
        self.remaining_commits = self.remaining_commits.saturating_sub(commits);
        self.remaining_files = self.remaining_files.saturating_sub(files);
    }
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
    let repo = Arc::new(repo);
    let request = visible_request(
        &state,
        &repo.record.id,
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
    let repo_for_inspection = repo.clone();
    let path_for_inspection = path.clone();
    let (file, old_content, new_content) = with_request_revision_store_repo(
        &state,
        &repo.incarnation(),
        &request,
        &revision,
        move |raw_repo, revision| {
            let inspected = request_revision_commit_files(
                raw_repo,
                &repo_for_inspection.policy,
                access,
                revision,
                &commit_oid,
            )?;
            let file = inspected
                .commit
                .files
                .into_iter()
                .find(|file| file.path == path_for_inspection)
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
    )
    .await?;
    Ok(Json(ReviewFileDiffResponse {
        path,
        kind: file.kind,
        old_mode: file.old_mode,
        new_mode: file.new_mode,
        old_content,
        new_content,
    }))
}

fn request_revision_commits(
    raw_repo: &FsPath,
    repo: &Repository,
    access: RepositoryAccess,
    revision: &RequestRevision,
    selected_commit: Option<&str>,
    limit: usize,
    file_limit: usize,
) -> Result<InspectedRequestCommits, ApiError> {
    let mut commit_oids = request_revision_commit_oids(raw_repo, revision, limit)?;
    let mut inspection_incomplete = commit_oids.len() > limit;
    if inspection_incomplete {
        commit_oids.drain(..commit_oids.len() - limit);
    }
    if let Some(selected_commit) = selected_commit
        && !commit_oids.iter().any(|commit| commit == selected_commit)
        && commit_belongs_to_revision(raw_repo, revision, selected_commit)?
    {
        if commit_oids.len() == limit {
            commit_oids.remove(0);
            inspection_incomplete = true;
        }
        commit_oids.insert(0, selected_commit.to_string());
    }
    let inspected = commit_oids.len();
    let mut inspection_order = (0..commit_oids.len()).rev().collect::<Vec<_>>();
    if let Some(selected_commit) = selected_commit
        && let Some(selected_index) = commit_oids.iter().position(|oid| oid == selected_commit)
    {
        inspection_order.retain(|index| *index != selected_index);
        inspection_order.insert(0, selected_index);
    }
    let mut remaining_files = file_limit;
    let mut visible = Vec::new();
    let mut file_budget_incomplete = false;
    let mut metadata_incomplete = false;
    let mut identity_only_indexes = Vec::new();
    for index in inspection_order {
        if remaining_files == 0 {
            identity_only_indexes.push(index);
            continue;
        }
        let commit = inspect_request_commit(raw_repo, &repo.policy, access, &commit_oids[index])?;
        metadata_incomplete |= commit.inspection == RequestRevisionInspectionState::Incomplete;
        if let Some(mut summary) = commit.commit {
            file_budget_incomplete |= truncate_commit_files(&mut summary, &mut remaining_files);
            visible.push((index, summary));
        }
    }
    if !identity_only_indexes.is_empty() {
        let identity_only_oids = identity_only_indexes
            .iter()
            .map(|index| commit_oids[*index].clone())
            .collect::<Vec<_>>();
        let identity_only = inspect_request_commits_identity_only(
            raw_repo,
            &repo.policy,
            access,
            &identity_only_oids,
        )?;
        for (index, commit) in identity_only_indexes.into_iter().zip(identity_only) {
            metadata_incomplete |= commit.inspection == RequestRevisionInspectionState::Incomplete;
            if let Some(summary) = commit.commit {
                file_budget_incomplete |= summary.files_truncated;
                visible.push((index, summary));
            }
        }
    }
    visible.sort_by_key(|(index, _)| *index);
    Ok(InspectedRequestCommits {
        visible: visible.into_iter().map(|(_, commit)| commit).collect(),
        inspected,
        files_listed: file_limit - remaining_files,
        inspection: if inspection_incomplete || file_budget_incomplete || metadata_incomplete {
            RequestRevisionInspectionState::Incomplete
        } else {
            RequestRevisionInspectionState::Complete
        },
    })
}

fn truncate_commit_files(
    commit: &mut RequestRevisionCommitResponse,
    remaining_files: &mut usize,
) -> bool {
    let listed = commit.files.len().min(*remaining_files);
    commit.files.truncate(listed);
    commit.files_truncated = listed < commit.change_count;
    *remaining_files = remaining_files.saturating_sub(listed);
    commit.files_truncated
}

struct InspectedRequestCommits {
    visible: Vec<RequestRevisionCommitResponse>,
    inspected: usize,
    files_listed: usize,
    inspection: RequestRevisionInspectionState,
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

struct VisibleRequestChanges {
    files: Vec<CommitFileResponse>,
    hidden: bool,
}

fn request_changes_from_repo_with_visibility(
    raw_repo: &FsPath,
    policy: &Policy,
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
        if !policy.can_read(&scope_path, access.can_read_private_files) {
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
            visibility: policy.effective_visibility(&scope_path).into(),
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
mod tests;
