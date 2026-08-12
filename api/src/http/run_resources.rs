use crate::{
    auth::scope::require_scope_user,
    error::ApiError,
    git::content::source_content_bytes,
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
use scope_domain::{
    repo_control::{RepoControlPath, classify_repo_control_path},
    runs::workflow::WorkflowRevision,
    store::StoredRepository,
};
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
        encode_history_cursor(last.run.created_at_unix, &last.run.id, workflow)
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
    let state = state.clone();
    let repo = repo.clone();
    tokio::task::spawn_blocking(move || current_workflows_blocking(&state, &repo))
        .await
        .map_err(|error| {
            ApiError::internal_message(format!("workflow loading task failed: {error}"))
        })?
}

fn current_workflows_blocking(
    state: &AppState,
    repo: &StoredRepository,
) -> Result<Vec<WorkflowRevision>, ApiError> {
    let manifest = repo.git_head.as_ref().map(|head| &head.manifest);
    let mut definitions = Vec::new();
    for (path, blob) in &repo.live_files {
        let Some(RepoControlPath::Workflow(workflow_path)) = classify_repo_control_path(path)
        else {
            continue;
        };
        definitions.push((
            workflow_path.as_str().to_string(),
            source_content_bytes(state, blob, manifest)?,
        ));
    }
    scope_run_config::parse_workflow_set(
        &repo.record.id,
        definitions
            .iter()
            .map(|(path, bytes)| (path.as_str(), bytes.as_slice())),
    )
    .map_err(ApiError::bad_request)
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
    let mut parts = value.splitn(4, ':');
    if parts.next() != Some("v1") {
        return Err(ApiError::bad_request("invalid run history cursor"));
    }
    let created_at_unix = parts
        .next()
        .and_then(|value| value.parse().ok())
        .ok_or_else(|| ApiError::bad_request("invalid run history cursor"))?;
    let run_id = parts
        .next()
        .filter(|value| !value.is_empty())
        .ok_or_else(|| ApiError::bad_request("invalid run history cursor"))?
        .to_string();
    let cursor_workflow = parts
        .next()
        .ok_or_else(|| ApiError::bad_request("invalid run history cursor"))?;
    if cursor_workflow != workflow.unwrap_or("*") {
        return Err(ApiError::bad_request(
            "run history cursor does not match the workflow filter",
        ));
    }
    Ok(RunHistoryCursor {
        created_at_unix,
        run_id,
    })
}

fn encode_history_cursor(created_at_unix: u64, run_id: &str, workflow: Option<&str>) -> String {
    format!("v1:{created_at_unix}:{run_id}:{}", workflow.unwrap_or("*"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn history_cursor_is_bound_to_the_workflow_filter() {
        let encoded = encode_history_cursor(42, "run_2", Some("checks"));
        assert_eq!(
            parse_history_cursor(&encoded, Some("checks")).unwrap(),
            RunHistoryCursor {
                created_at_unix: 42,
                run_id: "run_2".to_string(),
            }
        );
        assert!(parse_history_cursor(&encoded, None).is_err());
        assert!(parse_history_cursor("v1:42:run_2", Some("checks")).is_err());
    }
}
