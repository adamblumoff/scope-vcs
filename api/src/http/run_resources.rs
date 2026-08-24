use crate::{
    auth::scope::require_scope_user,
    error::ApiError,
    http::{
        responses::{
            RepositoryRunHistoryPageResponse, RepositoryRunWorkflowListResponse,
            RepositoryRunWorkflowResponse,
        },
        run_response::repository_run_summary,
    },
    repo_access::find_repo,
    state::AppState,
};
use axum::{
    Json,
    extract::{Path, Query, State},
    http::HeaderMap,
};
use scope_domain::{runs::workflow::WorkflowRevision, store::StoredRepository};
use scope_postgres::db::{RunHistoryCursor, RunHistoryPageQuery};
use serde::Deserialize;

const DEFAULT_RUN_HISTORY_PAGE_SIZE: usize = 20;
const MAX_RUN_HISTORY_PAGE_SIZE: usize = 100;

#[derive(Debug, Deserialize)]
pub(crate) struct RepositoryRunHistoryQuery {
    workflow: Option<String>,
    after: Option<String>,
    limit: Option<usize>,
}

pub(crate) async fn get_repository_run_workflows(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((owner, repo_name)): Path<(String, String)>,
) -> Result<Json<RepositoryRunWorkflowListResponse>, ApiError> {
    let repo = require_repository_member(&state, &headers, &owner, &repo_name).await?;
    let workflows = current_workflows(&state, &repo)
        .await?
        .into_iter()
        .map(|revision| RepositoryRunWorkflowResponse {
            key: revision.workflow().path().name().to_string(),
            name: revision.definition().name().to_string(),
            path: revision.workflow().path().as_str().to_string(),
            manual: revision.definition().triggers().manual(),
            push_main: revision.definition().triggers().push_main(),
            job_count: revision.definition().jobs().len(),
        })
        .collect();
    Ok(Json(RepositoryRunWorkflowListResponse { workflows }))
}

pub(crate) async fn get_repository_run_history(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((owner, repo_name)): Path<(String, String)>,
    Query(query): Query<RepositoryRunHistoryQuery>,
) -> Result<Json<RepositoryRunHistoryPageResponse>, ApiError> {
    let repo = require_repository_member(&state, &headers, &owner, &repo_name).await?;
    let workflow = query
        .workflow
        .as_deref()
        .map(str::trim)
        .filter(|key| !key.is_empty());
    let workflow_path = if let Some(key) = workflow {
        Some(
            current_workflows(&state, &repo)
                .await?
                .into_iter()
                .find(|revision| revision.workflow().path().name() == key)
                .map(|revision| revision.workflow().path().as_str().to_string())
                .ok_or_else(|| ApiError::bad_request("workflow is not defined on current main"))?,
        )
    } else {
        None
    };
    let after = query
        .after
        .as_deref()
        .map(|value| parse_history_cursor(value, workflow))
        .transpose()?;
    let limit = query
        .limit
        .unwrap_or(DEFAULT_RUN_HISTORY_PAGE_SIZE)
        .clamp(1, MAX_RUN_HISTORY_PAGE_SIZE);
    let mut entries = state
        .metadata
        .runs()
        .repository_run_history_page(RunHistoryPageQuery {
            repository_id: &repo.record.id,
            workflow_path: workflow_path.as_deref(),
            after: after.as_ref(),
            limit: (limit + 1) as u64,
        })
        .await?;
    let has_more = entries.len() > limit;
    entries.truncate(limit);
    let next_cursor = has_more.then(|| {
        let last = entries
            .last()
            .expect("a run history page with more results is non-empty");
        encode_history_cursor(last.creation_sequence, workflow)
    });
    let runs = entries
        .iter()
        .map(|entry| repository_run_summary(&entry.run, &entry.jobs))
        .collect::<Result<Vec<_>, _>>()?;
    Ok(Json(RepositoryRunHistoryPageResponse { runs, next_cursor }))
}

async fn current_workflows(
    state: &AppState,
    repo: &StoredRepository,
) -> Result<Vec<WorkflowRevision>, ApiError> {
    let catalog = state
        .metadata
        .repositories()
        .repository_workflow_catalog(&repo.record.id)
        .await
        .map_err(ApiError::from)?
        .ok_or_else(|| {
            ApiError::internal_message("repository workflow catalog is missing for current main")
        })?;
    let head = repo.git_head.as_ref().ok_or_else(|| {
        ApiError::internal_message("repository workflow catalog requires an accepted Git head")
    })?;
    catalog
        .verify_source(&repo.record.id, &head.head_oid, repo.record.change_version)
        .map_err(ApiError::internal)?;
    scope_run_config::parse_repository_workflow_catalog(&catalog).map_err(ApiError::bad_request)
}

async fn require_repository_member(
    state: &AppState,
    headers: &HeaderMap,
    owner: &str,
    repo_name: &str,
) -> Result<StoredRepository, ApiError> {
    let user = require_scope_user(state, headers).await?;
    let repo = find_repo(state, owner, repo_name).await?;
    if !repo.is_maintainer_user_id(&user.id) {
        return Err(ApiError::forbidden("repo membership required"));
    }
    Ok(repo)
}

fn parse_history_cursor(value: &str, workflow: Option<&str>) -> Result<RunHistoryCursor, ApiError> {
    let mut parts = value.splitn(3, ':');
    if parts.next() != Some("v2") {
        return Err(ApiError::bad_request("invalid run history cursor"));
    }
    let creation_sequence = parts
        .next()
        .and_then(|value| value.parse().ok())
        .ok_or_else(|| ApiError::bad_request("invalid run history cursor"))?;
    let cursor_workflow = parts
        .next()
        .ok_or_else(|| ApiError::bad_request("invalid run history cursor"))?;
    if cursor_workflow != workflow.unwrap_or("*") {
        return Err(ApiError::bad_request(
            "run history cursor does not match the workflow filter",
        ));
    }
    Ok(RunHistoryCursor { creation_sequence })
}

fn encode_history_cursor(creation_sequence: u64, workflow: Option<&str>) -> String {
    format!("v2:{creation_sequence}:{}", workflow.unwrap_or("*"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn history_cursor_is_bound_to_the_workflow_filter() {
        let encoded = encode_history_cursor(42, Some("checks"));
        assert_eq!(
            parse_history_cursor(&encoded, Some("checks")).unwrap(),
            RunHistoryCursor {
                creation_sequence: 42,
            }
        );
        assert!(parse_history_cursor(&encoded, None).is_err());
        assert!(parse_history_cursor("v1:42:checks", Some("checks")).is_err());
        assert!(parse_history_cursor("v2:42", Some("checks")).is_err());
    }
}
