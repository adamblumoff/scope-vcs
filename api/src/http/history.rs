use crate::{
    auth::scope::{optional_scope_user, principal_for_scope_user},
    domain::{
        commit_history::{CommitHistoryCommit, CommitHistoryFile, commit_history_view},
        policy::{Principal, PrincipalKind},
        store::StoredRepository,
    },
    error::ApiError,
    http::{
        projection_preview::ensure_projection_preview_access,
        responses::{
            CommitFileDiffRequest, CommitHistoryRequest, ProjectionPreviewAudience,
            ProjectionPreviewSource, ReviewFileDiffResponse, commit_detail_response,
            commit_history_response, pending_scope_path,
        },
    },
    object_store::{ObjectStore, source_blob_text},
    state::{AppState, find_repo},
};
use axum::{
    Json,
    extract::{Path, Query, State},
    http::HeaderMap,
};

pub(crate) async fn get_commit_history(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((owner, repo_name)): Path<(String, String)>,
    Query(input): Query<CommitHistoryRequest>,
) -> Result<Json<crate::http::responses::CommitHistoryResponse>, ApiError> {
    let (repo, audience) = repo_and_audience(&state, &headers, &owner, &repo_name, input).await?;
    let principal = history_principal(&repo, audience);
    let view = commit_history_view(&repo.policy, &repo.graph, &principal);

    Ok(Json(commit_history_response(audience, view)))
}

pub(crate) async fn get_commit_detail(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((owner, repo_name, commit_id)): Path<(String, String, String)>,
    Query(input): Query<CommitHistoryRequest>,
) -> Result<Json<crate::http::responses::CommitDetailResponse>, ApiError> {
    let (repo, audience) = repo_and_audience(&state, &headers, &owner, &repo_name, input).await?;
    let principal = history_principal(&repo, audience);
    let view = commit_history_view(&repo.policy, &repo.graph, &principal);
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
    let principal = history_principal(&repo, audience);
    let view = commit_history_view(&repo.policy, &repo.graph, &principal);
    let commit = commit_for_id(&view.commits, &commit_id)?;
    let path = pending_scope_path(&input.path)?;
    let file = commit
        .files
        .iter()
        .find(|file| file.path.as_str() == path.as_str())
        .ok_or_else(|| ApiError::not_found(format!("file {} not found", path.as_str())))?;

    Ok(Json(commit_file_diff_response(
        state.object_store.as_ref(),
        file,
    )?))
}

async fn repo_and_audience(
    state: &AppState,
    headers: &HeaderMap,
    owner: &str,
    repo_name: &str,
    input: CommitHistoryRequest,
) -> Result<(StoredRepository, ProjectionPreviewAudience), ApiError> {
    let repo = find_repo(state, owner, repo_name)?;
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

fn history_principal(repo: &StoredRepository, audience: ProjectionPreviewAudience) -> Principal {
    match audience {
        ProjectionPreviewAudience::Owner => Principal {
            id: repo.record.owner_user_id.clone(),
            kind: PrincipalKind::User,
        },
        ProjectionPreviewAudience::Public => Principal::public(),
    }
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
    store: &dyn ObjectStore,
    file: &CommitHistoryFile,
) -> Result<ReviewFileDiffResponse, ApiError> {
    Ok(ReviewFileDiffResponse {
        path: file.path.as_str().to_string(),
        kind: file.kind,
        old_content: file
            .old_content
            .as_ref()
            .map(|blob| source_blob_text(store, blob))
            .transpose()?,
        new_content: file
            .new_content
            .as_ref()
            .map(|blob| source_blob_text(store, blob))
            .transpose()?,
    })
}
