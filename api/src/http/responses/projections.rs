use crate::domain::{
    policy::{Principal, ScopePath, Visibility},
    projection_views::{
        ProjectionAudience, ProjectionPreviewCommit, ProjectionPreviewCommitVisibility,
        ProjectionPreviewFile, ProjectionPreviewSummary, ProjectionSource, ProjectionViewFile,
        files_for_visibility_update as domain_files_for_visibility_update,
        pending_import_files as domain_pending_import_files,
        pending_scope_path as domain_pending_scope_path, projection_preview,
    },
    store::StoredRepository,
};
use crate::error::ApiError;
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
#[cfg_attr(test, derive(ts_rs::TS))]
#[cfg_attr(test, ts(rename_all = "lowercase"))]
pub(crate) enum ProjectionPreviewAudience {
    Owner,
    Public,
}

impl From<ProjectionPreviewAudience> for ProjectionAudience {
    fn from(audience: ProjectionPreviewAudience) -> Self {
        match audience {
            ProjectionPreviewAudience::Owner => Self::Owner,
            ProjectionPreviewAudience::Public => Self::Public,
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
#[cfg_attr(test, derive(ts_rs::TS))]
#[cfg_attr(test, ts(rename_all = "lowercase"))]
pub(crate) enum ProjectionPreviewSource {
    Live,
    Review,
}

impl From<ProjectionPreviewSource> for ProjectionSource {
    fn from(source: ProjectionPreviewSource) -> Self {
        match source {
            ProjectionPreviewSource::Live => Self::Live,
            ProjectionPreviewSource::Review => Self::Review,
        }
    }
}

#[derive(Debug, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub(crate) struct ProjectionPreviewRequest {
    pub(crate) audience: ProjectionPreviewAudience,
    pub(crate) source: Option<ProjectionPreviewSource>,
}

#[derive(Debug, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub(crate) struct ProjectionPreviewResponse {
    pub(crate) audience: ProjectionPreviewAudience,
    pub(crate) source: ProjectionPreviewSource,
    pub(crate) repo_id: String,
    pub(crate) principal_id: String,
    pub(crate) head_oid: Option<String>,
    pub(crate) files: Vec<ProjectionPreviewFileResponse>,
    pub(crate) commits: Vec<ProjectionPreviewCommitResponse>,
    pub(crate) summary: ProjectionPreviewSummaryResponse,
}

#[derive(Debug, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub(crate) struct ProjectionPreviewFileResponse {
    pub(crate) path: String,
    pub(crate) oid: String,
    pub(crate) visibility: Visibility,
}

#[derive(Debug, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub(crate) struct ProjectionPreviewCommitResponse {
    pub(crate) projected_id: String,
    pub(crate) logical_commit_id: String,
    pub(crate) parent_projected_id: Option<String>,
    pub(crate) author: Option<String>,
    pub(crate) message: String,
    pub(crate) visibility: ProjectionPreviewCommitVisibilityResponse,
    pub(crate) change_count: usize,
}

#[derive(Debug, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub(crate) enum ProjectionPreviewCommitVisibilityResponse {
    FullyPublic,
    Mixed,
    FullyPrivate,
}

#[derive(Debug, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub(crate) struct ProjectionPreviewSummaryResponse {
    pub(crate) visible_files: usize,
    pub(crate) hidden_files: usize,
    pub(crate) visible_commits: usize,
    pub(crate) hidden_commits: usize,
}

#[derive(Debug, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub(crate) struct RepoFileResponse {
    pub(crate) path: String,
    pub(crate) oid: String,
    pub(crate) tracked: bool,
    pub(crate) visibility: Visibility,
}

pub(crate) fn projection_preview_response(
    repo: &StoredRepository,
    audience: ProjectionPreviewAudience,
    source: ProjectionPreviewSource,
    include_private_counts: bool,
) -> Result<ProjectionPreviewResponse, ApiError> {
    let preview = projection_preview(repo, audience.into(), include_private_counts);

    Ok(ProjectionPreviewResponse {
        audience,
        source,
        repo_id: preview.repo_id,
        principal_id: preview.principal_id,
        head_oid: preview.head_oid,
        files: preview
            .files
            .into_iter()
            .map(projection_preview_file_response)
            .collect(),
        commits: preview
            .commits
            .into_iter()
            .map(projection_preview_commit_response)
            .collect(),
        summary: projection_preview_summary_response(preview.summary),
    })
}

pub(crate) fn projection_file_responses(files: Vec<ProjectionViewFile>) -> Vec<RepoFileResponse> {
    files.into_iter().map(repo_file_response).collect()
}

pub(crate) fn pending_import_files(
    repo: &StoredRepository,
    principal: &Principal,
) -> Result<Vec<RepoFileResponse>, ApiError> {
    Ok(domain_pending_import_files(repo, principal)?
        .into_iter()
        .map(repo_file_response)
        .collect())
}

pub(crate) fn files_for_visibility_update(
    repo: &StoredRepository,
    principal: &Principal,
) -> Result<Vec<RepoFileResponse>, ApiError> {
    Ok(domain_files_for_visibility_update(repo, principal)?
        .into_iter()
        .map(repo_file_response)
        .collect())
}

pub(crate) fn pending_scope_path(path: &str) -> Result<ScopePath, ApiError> {
    domain_pending_scope_path(path)
}

fn projection_preview_file_response(file: ProjectionPreviewFile) -> ProjectionPreviewFileResponse {
    ProjectionPreviewFileResponse {
        path: file.path.as_str().to_string(),
        oid: file.oid,
        visibility: file.visibility,
    }
}

fn projection_preview_commit_response(
    commit: ProjectionPreviewCommit,
) -> ProjectionPreviewCommitResponse {
    ProjectionPreviewCommitResponse {
        projected_id: commit.projected_id,
        logical_commit_id: commit.logical_commit_id,
        parent_projected_id: commit.parent_projected_id,
        author: commit.author,
        message: commit.message,
        visibility: projection_preview_commit_visibility_response(commit.visibility),
        change_count: commit.change_count,
    }
}

fn projection_preview_commit_visibility_response(
    visibility: ProjectionPreviewCommitVisibility,
) -> ProjectionPreviewCommitVisibilityResponse {
    match visibility {
        ProjectionPreviewCommitVisibility::FullyPublic => {
            ProjectionPreviewCommitVisibilityResponse::FullyPublic
        }
        ProjectionPreviewCommitVisibility::Mixed => {
            ProjectionPreviewCommitVisibilityResponse::Mixed
        }
        ProjectionPreviewCommitVisibility::FullyPrivate => {
            ProjectionPreviewCommitVisibilityResponse::FullyPrivate
        }
    }
}

fn projection_preview_summary_response(
    summary: ProjectionPreviewSummary,
) -> ProjectionPreviewSummaryResponse {
    ProjectionPreviewSummaryResponse {
        visible_files: summary.visible_files,
        hidden_files: summary.hidden_files,
        visible_commits: summary.visible_commits,
        hidden_commits: summary.hidden_commits,
    }
}

fn repo_file_response(file: ProjectionViewFile) -> RepoFileResponse {
    RepoFileResponse {
        path: file.path.as_str().to_string(),
        oid: file.oid,
        tracked: file.tracked,
        visibility: file.visibility,
    }
}
