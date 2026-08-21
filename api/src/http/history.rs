use crate::{
    auth::scope::{optional_scope_user, principal_for_scope_user},
    error::ApiError,
    http::{
        file_diffs::review_file_diff_response_for_blobs,
        projection_preview::ensure_projection_preview_access,
        responses::{
            HistoryEntryFileDiffRequest, HistoryEntryRequest, HistoryPageRequest,
            ProjectionPreviewAudience, ProjectionPreviewSource, ReviewFileDiffResponse,
            history_entry_detail_response, history_page_response, repo_scope_path,
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
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use scope_domain::{
    history::{HistoryEntry, HistoryEntryFile, HistoryView, history_view_from_projection},
    projection::{ProjectionViewKey, project_graph},
    store::StoredRepository,
};
use serde::{Deserialize, Serialize};

const HISTORY_PAGE_SIZE: usize = 50;
const HISTORY_CURSOR_VERSION: u8 = 1;

#[derive(Debug, Deserialize, Serialize)]
struct HistoryCursor {
    version: u8,
    repo_id: String,
    audience: ProjectionPreviewAudience,
    boundary_id: String,
}

pub(crate) async fn get_history_page(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((owner, repo_name)): Path<(String, String)>,
    Query(input): Query<HistoryPageRequest>,
) -> Result<Json<crate::http::responses::HistoryPageResponse>, ApiError> {
    let (repo, audience) =
        repo_and_audience(&state, &headers, &owner, &repo_name, input.audience).await?;
    let view = history_view_for_repo(&repo, audience)?;
    let boundary_id = input
        .before
        .as_deref()
        .map(|cursor| parse_history_cursor(cursor, &repo.record.id, audience))
        .transpose()?;
    let (entries, has_more) = history_page(&view, boundary_id.as_deref())?;
    let next_cursor = if has_more {
        entries
            .last()
            .map(|entry| encode_history_cursor(&repo.record.id, audience, &entry.id))
            .transpose()?
    } else {
        None
    };

    Ok(Json(history_page_response(
        audience,
        &view,
        entries,
        next_cursor,
    )))
}

pub(crate) async fn get_history_entry(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((owner, repo_name, entry_id)): Path<(String, String, String)>,
    Query(input): Query<HistoryEntryRequest>,
) -> Result<Json<crate::http::responses::HistoryEntryDetailResponse>, ApiError> {
    let (repo, audience) =
        repo_and_audience(&state, &headers, &owner, &repo_name, input.audience).await?;
    let view = history_view_for_repo(&repo, audience)?;
    let entry = history_entry_for_id(&view.entries, &entry_id)?;

    Ok(Json(history_entry_detail_response(audience, &view, entry)))
}

pub(crate) async fn get_history_entry_file_diff(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((owner, repo_name, entry_id)): Path<(String, String, String)>,
    Query(input): Query<HistoryEntryFileDiffRequest>,
) -> Result<Json<ReviewFileDiffResponse>, ApiError> {
    let (repo, audience) =
        repo_and_audience(&state, &headers, &owner, &repo_name, input.audience).await?;
    let view = history_view_for_repo(&repo, audience)?;
    let entry = history_entry_for_id(&view.entries, &entry_id)?;
    let path = repo_scope_path(&input.path)?;
    let file = entry
        .files
        .iter()
        .find(|file| file.path.as_str() == path.as_str())
        .ok_or_else(|| ApiError::not_found(format!("file {} not found", path.as_str())))?;

    Ok(Json(history_entry_file_diff_response(&state, &repo, file)?))
}

async fn repo_and_audience(
    state: &AppState,
    headers: &HeaderMap,
    owner: &str,
    repo_name: &str,
    requested_audience: Option<ProjectionPreviewAudience>,
) -> Result<(StoredRepository, ProjectionPreviewAudience), ApiError> {
    let repo = find_repo(state, owner, repo_name).await?;
    let user = optional_scope_user(state, headers).await?;
    let requester = principal_for_scope_user(&repo, user.as_ref());
    let audience = requested_audience.unwrap_or_else(|| {
        if repo.access_for_principal(&requester).can_read_private_files {
            ProjectionPreviewAudience::Private
        } else {
            ProjectionPreviewAudience::Public
        }
    });
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

fn history_view_for_repo(
    repo: &StoredRepository,
    audience: ProjectionPreviewAudience,
) -> Result<HistoryView, ApiError> {
    let projection = project_graph(
        &repo.graph,
        &repo.visibility_events,
        history_view_key(audience),
    );
    if projection.preserves_git_commits() {
        return Err(ApiError::not_implemented(
            "history is unavailable for preserved public request commits until native per-commit diffs are represented accurately",
        ));
    }
    Ok(history_view_from_projection(
        projection,
        &repo.graph,
        &repo.visibility_events,
    ))
}

fn history_page<'a>(
    view: &'a HistoryView,
    boundary_id: Option<&str>,
) -> Result<(&'a [HistoryEntry], bool), ApiError> {
    let start = match boundary_id {
        Some(boundary_id) => view
            .entries
            .iter()
            .position(|entry| entry.id == boundary_id)
            .map(|index| index + 1)
            .ok_or_else(|| {
                ApiError::bad_request("history cursor boundary is no longer available")
            })?,
        None => 0,
    };
    let lookahead_end = (start + HISTORY_PAGE_SIZE + 1).min(view.entries.len());
    let has_more = lookahead_end - start > HISTORY_PAGE_SIZE;
    let page_end = (start + HISTORY_PAGE_SIZE).min(view.entries.len());
    Ok((&view.entries[start..page_end], has_more))
}

fn encode_history_cursor(
    repo_id: &str,
    audience: ProjectionPreviewAudience,
    boundary_id: &str,
) -> Result<String, ApiError> {
    let cursor = HistoryCursor {
        version: HISTORY_CURSOR_VERSION,
        repo_id: repo_id.to_string(),
        audience,
        boundary_id: boundary_id.to_string(),
    };
    let encoded = serde_json::to_vec(&cursor).map_err(ApiError::internal)?;
    Ok(URL_SAFE_NO_PAD.encode(encoded))
}

fn parse_history_cursor(
    value: &str,
    repo_id: &str,
    audience: ProjectionPreviewAudience,
) -> Result<String, ApiError> {
    let invalid = || ApiError::bad_request("invalid history cursor");
    let decoded = URL_SAFE_NO_PAD.decode(value).map_err(|_| invalid())?;
    let cursor: HistoryCursor = serde_json::from_slice(&decoded).map_err(|_| invalid())?;
    if cursor.version != HISTORY_CURSOR_VERSION {
        return Err(invalid());
    }
    if cursor.repo_id != repo_id || cursor.audience != audience {
        return Err(ApiError::bad_request(
            "history cursor does not match the repository and audience",
        ));
    }
    Ok(cursor.boundary_id)
}

fn history_entry_for_id<'a>(
    entries: &'a [HistoryEntry],
    entry_id: &str,
) -> Result<&'a HistoryEntry, ApiError> {
    entries
        .iter()
        .find(|entry| entry.id == entry_id || entry.source_id == entry_id)
        .ok_or_else(|| ApiError::not_found(format!("history entry {entry_id} not found")))
}

fn history_entry_file_diff_response(
    state: &AppState,
    repo: &StoredRepository,
    file: &HistoryEntryFile,
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cursor_is_bound_to_repository_and_audience() {
        let encoded =
            encode_history_cursor("owner/repo", ProjectionPreviewAudience::Public, "entry-50")
                .unwrap();

        assert_eq!(
            parse_history_cursor(&encoded, "owner/repo", ProjectionPreviewAudience::Public)
                .unwrap(),
            "entry-50"
        );
        assert!(
            parse_history_cursor(&encoded, "owner/repo", ProjectionPreviewAudience::Private)
                .is_err()
        );
        assert!(
            parse_history_cursor(&encoded, "owner/other", ProjectionPreviewAudience::Public)
                .is_err()
        );
        assert!(
            parse_history_cursor(
                "not-a-cursor",
                "owner/repo",
                ProjectionPreviewAudience::Public
            )
            .is_err()
        );
    }
}
