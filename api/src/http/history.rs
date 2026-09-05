use crate::{
    auth::scope::optional_scope_user,
    error::ApiError,
    http::{
        file_diffs::review_file_diff_response_for_blobs,
        responses::{
            HistoryEntryFileDiffRequest, HistoryEntryRequest, HistoryPageRequest,
            ProjectionPreviewAudience, ReviewFileDiffResponse, history_entry_detail_response,
            history_page_response, repo_scope_path,
        },
    },
    repo_access::find_read_access,
    state::AppState,
};
use axum::{
    Json,
    extract::{Path, Query, State},
    http::HeaderMap,
};
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use scope_domain::{
    history::{HistoryEntry, HistoryEntryFile},
    projection::ProjectionViewKey,
    repository::access::{RepositoryAccessContext, RepositoryActor},
};
use serde::{Deserialize, Serialize};

const HISTORY_PAGE_SIZE: usize = 50;
const HISTORY_CURSOR_VERSION: u8 = 2;

#[derive(Debug, Deserialize, Serialize)]
struct HistoryCursor {
    version: u8,
    repo_id: String,
    audience: ProjectionPreviewAudience,
    generation: String,
    boundary_position: u64,
}

pub(crate) async fn get_history_page(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((owner, repo_name)): Path<(String, String)>,
    Query(input): Query<HistoryPageRequest>,
) -> Result<Json<crate::http::responses::HistoryPageResponse>, ApiError> {
    let (repo, audience) =
        repo_and_audience(&state, &headers, &owner, &repo_name, input.audience).await?;
    let boundary = input
        .before
        .as_deref()
        .map(|cursor| parse_history_cursor(cursor, &repo.record.id, audience))
        .transpose()?;
    let page = state
        .metadata
        .repositories()
        .repository_history_page(scope_postgres::db::RepositoryHistoryQuery {
            incarnation: &repo.incarnation(),
            version: repo.record.change_version,
            audience: history_view_key(audience),
            before: boundary.as_ref(),
            entry_source_id: None,
            limit: HISTORY_PAGE_SIZE as u64,
        })
        .await?;
    ensure_history_available(page.available)?;
    let view = page.view;
    let entries = view.entries.as_slice();
    let next_cursor = page
        .next_boundary
        .as_ref()
        .map(|boundary| encode_history_cursor(&repo.record.id, audience, boundary))
        .transpose()?;

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
    let page = state
        .metadata
        .repositories()
        .repository_history_page(scope_postgres::db::RepositoryHistoryQuery {
            incarnation: &repo.incarnation(),
            version: repo.record.change_version,
            audience: history_view_key(audience),
            before: None,
            entry_source_id: Some(&entry_id),
            limit: 1,
        })
        .await?;
    ensure_history_available(page.available)?;
    let view = page.view;
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
    let page = state
        .metadata
        .repositories()
        .repository_history_page(scope_postgres::db::RepositoryHistoryQuery {
            incarnation: &repo.incarnation(),
            version: repo.record.change_version,
            audience: history_view_key(audience),
            before: None,
            entry_source_id: Some(&entry_id),
            limit: 1,
        })
        .await?;
    ensure_history_available(page.available)?;
    let view = page.view;
    let entry = history_entry_for_id(&view.entries, &entry_id)?;
    let path = repo_scope_path(&input.path)?;
    let file = entry
        .files
        .iter()
        .find(|file| file.path.as_str() == path.as_str())
        .ok_or_else(|| ApiError::not_found(format!("file {} not found", path.as_str())))?;

    Ok(Json(
        history_entry_file_diff_response(&state, &repo, file).await?,
    ))
}

async fn repo_and_audience(
    state: &AppState,
    headers: &HeaderMap,
    owner: &str,
    repo_name: &str,
    requested_audience: Option<ProjectionPreviewAudience>,
) -> Result<(RepositoryAccessContext, ProjectionPreviewAudience), ApiError> {
    let user = optional_scope_user(state, headers).await?;
    let repo = find_read_access(
        state,
        owner,
        repo_name,
        user.as_ref().map(|user| user.id.as_str()),
    )
    .await?;
    let audience = requested_audience.unwrap_or(if repo.access.can_read_private_files {
        ProjectionPreviewAudience::Private
    } else {
        ProjectionPreviewAudience::Public
    });
    if audience == ProjectionPreviewAudience::Private
        && repo.access.actor == RepositoryActor::Public
    {
        return Err(ApiError::forbidden("repo membership required"));
    }
    Ok((repo, audience))
}

fn history_view_key(audience: ProjectionPreviewAudience) -> ProjectionViewKey {
    match audience {
        ProjectionPreviewAudience::Private => ProjectionViewKey::Private,
        ProjectionPreviewAudience::Public => ProjectionViewKey::Public,
    }
}

fn ensure_history_available(available: bool) -> Result<(), ApiError> {
    if available {
        Ok(())
    } else {
        Err(ApiError::not_implemented(
            "history is unavailable for preserved public request commits until native per-commit diffs are represented accurately",
        ))
    }
}

fn encode_history_cursor(
    repo_id: &str,
    audience: ProjectionPreviewAudience,
    boundary: &scope_postgres::db::RepositoryHistoryBoundary,
) -> Result<String, ApiError> {
    let cursor = HistoryCursor {
        version: HISTORY_CURSOR_VERSION,
        repo_id: repo_id.to_string(),
        audience,
        generation: boundary.generation.clone(),
        boundary_position: boundary.position,
    };
    let encoded = serde_json::to_vec(&cursor).map_err(ApiError::internal)?;
    Ok(URL_SAFE_NO_PAD.encode(encoded))
}

fn parse_history_cursor(
    value: &str,
    repo_id: &str,
    audience: ProjectionPreviewAudience,
) -> Result<scope_postgres::db::RepositoryHistoryBoundary, ApiError> {
    let invalid = || ApiError::bad_request("invalid history cursor");
    let decoded = URL_SAFE_NO_PAD.decode(value).map_err(|_| invalid())?;
    let cursor: HistoryCursor = serde_json::from_slice(&decoded).map_err(|_| invalid())?;
    if cursor.version != HISTORY_CURSOR_VERSION || cursor.boundary_position > i64::MAX as u64 {
        return Err(invalid());
    }
    if cursor.repo_id != repo_id || cursor.audience != audience {
        return Err(ApiError::bad_request(
            "history cursor does not match the repository and audience",
        ));
    }
    Ok(scope_postgres::db::RepositoryHistoryBoundary {
        generation: cursor.generation,
        position: cursor.boundary_position,
    })
}

fn history_entry_for_id<'a>(
    entries: &'a [HistoryEntry],
    entry_source_id: &str,
) -> Result<&'a HistoryEntry, ApiError> {
    entries
        .iter()
        .find(|entry| entry.source_id == entry_source_id)
        .ok_or_else(|| ApiError::not_found(format!("history entry {entry_source_id} not found")))
}

async fn history_entry_file_diff_response(
    state: &AppState,
    repo: &RepositoryAccessContext,
    file: &HistoryEntryFile,
) -> Result<ReviewFileDiffResponse, ApiError> {
    let (head, spans) = state
        .metadata
        .repositories()
        .repository_content_source(&repo.incarnation())
        .await?;
    review_file_diff_response_for_blobs(
        state,
        head.as_ref()
            .map(|head| (repo.incarnation(), head, spans.as_slice())),
        file.path.as_str().to_string(),
        file.kind,
        file.old_content.as_ref(),
        file.new_content.as_ref(),
    )
    .await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cursor_is_bound_to_repository_and_audience() {
        let boundary = scope_postgres::db::RepositoryHistoryBoundary {
            generation: "generation-1".into(),
            position: 50,
        };
        let encoded =
            encode_history_cursor("owner/repo", ProjectionPreviewAudience::Public, &boundary)
                .unwrap();

        assert_eq!(
            parse_history_cursor(&encoded, "owner/repo", ProjectionPreviewAudience::Public)
                .unwrap(),
            boundary
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
        let mut cursor: HistoryCursor =
            serde_json::from_slice(&URL_SAFE_NO_PAD.decode(&encoded).unwrap()).unwrap();
        cursor.boundary_position = u64::MAX;
        let overflow = URL_SAFE_NO_PAD.encode(serde_json::to_vec(&cursor).unwrap());
        assert!(
            parse_history_cursor(&overflow, "owner/repo", ProjectionPreviewAudience::Public)
                .is_err()
        );
    }
}
