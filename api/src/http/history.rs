use crate::{
    auth::scope::{optional_scope_user, principal_for_scope_user},
    error::ApiError,
    http::{
        file_diffs::review_file_diff_response_for_blobs,
        projection_preview::ensure_projection_preview_access,
        responses::{
            CommitFileDiffRequest, CommitHistoryRequest, ProjectionPreviewAudience,
            ProjectionPreviewSource, ReviewFileDiffResponse, commit_detail_response,
            commit_history_response, repo_scope_path,
        },
    },
    repo_access::find_repo,
    state::AppState,
};
use axum::{
    Json,
    extract::{Path, Query, State},
    http::HeaderMap,
};
use scope_domain::{
    commit_history::{
        CommitHistoryCommit, CommitHistoryFile, CommitHistoryView,
        commit_history_view_from_projection,
    },
    projection::{ProjectionViewKey, project_graph},
    store::StoredRepository,
};

pub(crate) async fn get_commit_history(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((owner, repo_name)): Path<(String, String)>,
    Query(input): Query<CommitHistoryRequest>,
) -> Result<Json<crate::http::responses::CommitHistoryResponse>, ApiError> {
    let (repo, audience) = repo_and_audience(&state, &headers, &owner, &repo_name, input).await?;
    let view = commit_history_view_for_repo(&repo, audience)?;

    Ok(Json(commit_history_response(audience, view)))
}

pub(crate) async fn get_commit_detail(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((owner, repo_name, commit_id)): Path<(String, String, String)>,
    Query(input): Query<CommitHistoryRequest>,
) -> Result<Json<crate::http::responses::CommitDetailResponse>, ApiError> {
    let (repo, audience) = repo_and_audience(&state, &headers, &owner, &repo_name, input).await?;
    let view = commit_history_view_for_repo(&repo, audience)?;
    let commit = commit_for_id(&view.commits, &commit_id)?;

    Ok(Json(commit_detail_response(audience, &view, commit)))
}

pub(crate) async fn get_commit_file_diff(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((owner, repo_name, commit_id)): Path<(String, String, String)>,
    Query(input): Query<CommitFileDiffRequest>,
) -> Result<Json<ReviewFileDiffResponse>, ApiError> {
    let request = CommitHistoryRequest {
        audience: input.audience,
    };
    let (repo, audience) = repo_and_audience(&state, &headers, &owner, &repo_name, request).await?;
    let view = commit_history_view_for_repo(&repo, audience)?;
    let commit = commit_for_id(&view.commits, &commit_id)?;
    let path = repo_scope_path(&input.path)?;
    let file = commit
        .files
        .iter()
        .find(|file| file.path.as_str() == path.as_str())
        .ok_or_else(|| ApiError::not_found(format!("file {} not found", path.as_str())))?;

    Ok(Json(commit_file_diff_response(&state, &repo, file)?))
}

async fn repo_and_audience(
    state: &AppState,
    headers: &HeaderMap,
    owner: &str,
    repo_name: &str,
    input: CommitHistoryRequest,
) -> Result<(StoredRepository, ProjectionPreviewAudience), ApiError> {
    let repo = find_repo(state, owner, repo_name).await?;
    let audience = input.audience.unwrap_or(ProjectionPreviewAudience::Public);
    let user = optional_scope_user(state, headers).await?;
    let requester = principal_for_scope_user(&repo, user.as_ref());
    ensure_projection_preview_access(
        state,
        &repo,
        &requester,
        audience,
        ProjectionPreviewSource::Live,
    )?;

    Ok((repo, audience))
}

fn history_view_key(audience: ProjectionPreviewAudience) -> ProjectionViewKey {
    match audience {
        ProjectionPreviewAudience::Private => ProjectionViewKey::Private,
        ProjectionPreviewAudience::Public => ProjectionViewKey::Public,
    }
}

fn commit_history_view_for_repo(
    repo: &StoredRepository,
    audience: ProjectionPreviewAudience,
) -> Result<CommitHistoryView, ApiError> {
    let projection = project_graph(
        &repo.graph,
        &repo.visibility_events,
        history_view_key(audience),
    );
    if projection.preserves_git_commits() {
        return Err(ApiError::not_implemented(
            "commit history is unavailable for preserved public request commits until native per-commit diffs are represented accurately",
        ));
    }
    Ok(commit_history_view_from_projection(projection))
}

fn commit_for_id<'a>(
    commits: &'a [CommitHistoryCommit],
    commit_id: &str,
) -> Result<&'a CommitHistoryCommit, ApiError> {
    commits
        .iter()
        .find(|commit| commit.projected_id == commit_id || commit.logical_commit_id == commit_id)
        .ok_or_else(|| ApiError::not_found(format!("commit {commit_id} not found")))
}

fn commit_file_diff_response(
    state: &AppState,
    repo: &StoredRepository,
    file: &CommitHistoryFile,
) -> Result<ReviewFileDiffResponse, ApiError> {
    review_file_diff_response_for_blobs(
        state,
        repo.git_head.as_ref().map(|head| &head.manifest),
        file.path.as_str().to_string(),
        file.kind,
        file.old_content.as_ref(),
        file.new_content.as_ref(),
    )
}
