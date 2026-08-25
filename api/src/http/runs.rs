use crate::{
    auth::scope::require_scope_user,
    error::ApiError,
    http::responses::{
        RepositoryRunLogResponse, RepositoryRunStepLogPageResponse, git_oid_request,
    },
    http::run_detail_response::build_run_detail_response,
    http::run_response::{repository_run_summary, run_response},
    state::AppState,
    use_cases::{
        run_control::{
            ManualRunCommand, cancel_run as cancel_run_control,
            create_manual_run as create_manual_run_control, retry_run as retry_run_control,
        },
        run_inspection::{
            inspect_run, inspect_run_detail, inspect_run_step_logs, require_repo_member,
        },
    },
};
use axum::{
    Json,
    body::{Body, to_bytes},
    extract::{Path, Query, State},
    http::HeaderMap,
};
use scope_api_contract::{
    CreateManualRunQuery, PushTriggerCheckResponse, PushTriggerEvaluationResponse,
    RepositoryRunDetailResponse, RunResponse,
};
use serde::Deserialize;
use std::collections::BTreeMap;

const MAX_MANUAL_BUNDLE_BYTES: usize = 128 * 1024 * 1024;
const REPOSITORY_STEP_LOG_LIMIT: u64 = 128;

pub(crate) async fn create_manual_run(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((owner, repo_name)): Path<(String, String)>,
    Query(query): Query<CreateManualRunQuery>,
    body: Body,
) -> Result<Json<RunResponse>, ApiError> {
    let user = require_scope_user(&state, &headers).await?;
    let repo = require_repo_member(&state, &user.id, &owner, &repo_name).await?;
    validate_request_id(&query.request_id)?;
    let git_oid = git_oid_request("git_oid", &query.git_oid)?;
    let bundle = to_bytes(body, MAX_MANUAL_BUNDLE_BYTES)
        .await
        .map_err(|error| ApiError::payload_too_large(format!("run bundle is too large: {error}")))?
        .to_vec();
    let inspected = create_manual_run_control(
        &state,
        ManualRunCommand {
            repository_id: repo.record.id,
            user_id: user.id,
            request_id: query.request_id,
            git_oid,
            workflow_name: query.workflow,
            bundle,
        },
    )
    .await?;
    Ok(Json(run_response(
        &inspected.run,
        &inspected.jobs,
        inspected.logs_truncated,
    )?))
}

pub(crate) async fn get_run(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((owner, repo_name, run_id)): Path<(String, String, String)>,
) -> Result<Json<RunResponse>, ApiError> {
    let user = require_scope_user(&state, &headers).await?;
    let snapshot = inspect_run(&state, &user.id, &owner, &repo_name, &run_id).await?;
    Ok(Json(run_response(
        &snapshot.run,
        &snapshot.jobs,
        snapshot.logs_truncated,
    )?))
}

pub(crate) async fn get_repository_run_detail(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((owner, repo_name, run_id)): Path<(String, String, String)>,
) -> Result<Json<RepositoryRunDetailResponse>, ApiError> {
    let user = require_scope_user(&state, &headers).await?;
    let detail = inspect_run_detail(&state, &user.id, &owner, &repo_name, &run_id).await?;
    let run = repository_run_summary(&detail.run, &detail.jobs)?;
    Ok(Json(build_run_detail_response(detail, run)?))
}

#[derive(Debug, Deserialize)]
pub(crate) struct RepositoryStepLogsQuery {
    #[serde(default)]
    after: u64,
}

pub(crate) async fn get_repository_run_step_logs(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((owner, repo_name, run_id, attempt_id, step_index)): Path<(
        String,
        String,
        String,
        String,
        u32,
    )>,
    Query(query): Query<RepositoryStepLogsQuery>,
) -> Result<Json<RepositoryRunStepLogPageResponse>, ApiError> {
    let user = require_scope_user(&state, &headers).await?;
    let page = inspect_run_step_logs(
        &state,
        &user.id,
        &owner,
        &repo_name,
        &run_id,
        &attempt_id,
        step_index,
        query.after,
        REPOSITORY_STEP_LOG_LIMIT,
    )
    .await?;

    Ok(Json(RepositoryRunStepLogPageResponse {
        logs: page
            .logs
            .into_iter()
            .map(|stored| RepositoryRunLogResponse {
                position: stored.position,
                sequence: stored.chunk.sequence,
                text: stored.chunk.text,
                created_at_unix: stored.chunk.created_at_unix,
            })
            .collect(),
        next_after: page.next_after,
        logs_truncated: page.logs_truncated,
    }))
}

pub(crate) async fn get_push_trigger_evaluation(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((owner, repo_name, head_oid)): Path<(String, String, String)>,
) -> Result<Json<PushTriggerEvaluationResponse>, ApiError> {
    let user = require_scope_user(&state, &headers).await?;
    let repo = require_repo_member(&state, &user.id, &owner, &repo_name).await?;
    let head_oid = git_oid_request("head_oid", &head_oid)?;
    let evaluation = state
        .metadata
        .runs()
        .push_trigger_evaluation(&repo.record.id, &head_oid)
        .await?
        .ok_or_else(|| ApiError::not_found("push trigger evaluation not found"))?;
    let run_ids = evaluation
        .checks
        .iter()
        .map(|check| check.run_id.clone())
        .collect::<Vec<_>>();
    let runs_store = state.metadata.runs();
    let (runs, mut jobs, truncated_run_ids) = tokio::try_join!(
        runs_store.runs_by_ids(&run_ids),
        runs_store.run_jobs_by_ids(&run_ids),
        runs_store.run_ids_with_truncated_logs(&run_ids),
    )?;
    let mut runs = runs
        .into_iter()
        .map(|run| (run.id.clone(), run))
        .collect::<BTreeMap<_, _>>();
    if run_ids.iter().any(|run_id| !runs.contains_key(run_id)) {
        return Err(ApiError::not_found(
            "push trigger evaluation history has expired",
        ));
    }
    let checks = evaluation
        .checks
        .into_iter()
        .map(|check| {
            let run = runs
                .remove(&check.run_id)
                .ok_or_else(|| ApiError::internal_message("push trigger check run is missing"))?;
            let run_jobs = jobs
                .remove(&run.id)
                .filter(|jobs| !jobs.is_empty())
                .ok_or_else(|| {
                    ApiError::internal_message(
                        "push trigger check run is missing its persisted jobs",
                    )
                })?;
            Ok(PushTriggerCheckResponse {
                workflow_path: check.workflow_path,
                workflow_name: check.workflow_name,
                run: run_response(&run, &run_jobs, truncated_run_ids.contains(&run.id))?,
            })
        })
        .collect::<Result<Vec<_>, ApiError>>()?;
    Ok(Json(PushTriggerEvaluationResponse {
        change_version: evaluation.change_version,
        head_oid: evaluation.head_oid,
        state: match evaluation.state {
            scope_domain::runs::trigger::PushTriggerEvaluationState::Pending => {
                scope_api_contract::PushTriggerEvaluationState::Pending
            }
            scope_domain::runs::trigger::PushTriggerEvaluationState::Succeeded => {
                scope_api_contract::PushTriggerEvaluationState::Succeeded
            }
            scope_domain::runs::trigger::PushTriggerEvaluationState::ConfigurationError => {
                scope_api_contract::PushTriggerEvaluationState::ConfigurationError
            }
            scope_domain::runs::trigger::PushTriggerEvaluationState::Failed => {
                scope_api_contract::PushTriggerEvaluationState::Failed
            }
        },
        message: evaluation.message,
        checks,
    }))
}

pub(crate) async fn cancel_run(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((owner, repo_name, run_id)): Path<(String, String, String)>,
) -> Result<Json<RunResponse>, ApiError> {
    let user = require_scope_user(&state, &headers).await?;
    let inspected = cancel_run_control(&state, &user.id, &owner, &repo_name, &run_id).await?;
    Ok(Json(run_response(
        &inspected.run,
        &inspected.jobs,
        inspected.logs_truncated,
    )?))
}

pub(crate) async fn retry_run(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((owner, repo_name, run_id)): Path<(String, String, String)>,
) -> Result<Json<RunResponse>, ApiError> {
    let user = require_scope_user(&state, &headers).await?;
    let inspected = retry_run_control(&state, &user.id, &owner, &repo_name, &run_id).await?;
    Ok(Json(run_response(
        &inspected.run,
        &inspected.jobs,
        inspected.logs_truncated,
    )?))
}

fn validate_request_id(request_id: &str) -> Result<(), ApiError> {
    if request_id.len() != 32 || !request_id.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(ApiError::bad_request(
            "request_id must be a 32-character hexadecimal value",
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use scope_domain::{
        content::SourceBlob,
        content_ref::ContentRef,
        runs::{
            attempt::MAX_RUN_ATTEMPTS,
            job::{RunJob, RunJobState},
            run::{Run, RunState},
            source::{RunSource, RunTrigger},
            workflow::{
                definition::WorkflowJobId,
                identity::{WorkflowIdentity, WorkflowPath},
            },
        },
    };

    #[test]
    fn run_summary_allows_retry_when_every_job_has_capacity() {
        let run = terminal_run();
        let available = terminal_job(1);
        assert!(
            repository_run_summary(&run, &[available])
                .unwrap()
                .can_retry
        );
    }

    #[test]
    fn run_summary_hides_retry_when_any_job_is_exhausted() {
        let run = terminal_run();
        let exhausted = terminal_job(MAX_RUN_ATTEMPTS);
        assert!(
            !repository_run_summary(&run, &[exhausted])
                .unwrap()
                .can_retry
        );
    }

    fn terminal_run() -> Run {
        Run::restore(
            "run-summary",
            "manual:summary",
            WorkflowIdentity::new(
                "owner/repo",
                WorkflowPath::parse("/.scope/runs/checks.yml").unwrap(),
            )
            .unwrap(),
            "a".repeat(64),
            RunTrigger::Manual,
            Some("user-1".to_string()),
            RunSource::ephemeral_git_bundle(SourceBlob {
                content_ref: ContentRef::git_bundle_sha256("b".repeat(64)),
                sha256: "b".repeat(64),
                git_oid: "c".repeat(40),
                git_file_mode: "100644".to_string(),
                size_bytes: 1,
            })
            .unwrap(),
            RunState::Failed,
            false,
            1,
            2,
            Some(2),
        )
        .unwrap()
    }

    fn terminal_job(last_attempt_number: u32) -> RunJob {
        RunJob::restore(
            "run-summary",
            WorkflowJobId::parse("checks").unwrap(),
            scope_domain::runs::image::PinnedContainerImage::parse(format!(
                "rust@sha256:{}",
                "d".repeat(64)
            ))
            .unwrap(),
            RunJobState::Failed,
            last_attempt_number,
            None,
            1,
            2,
            Some(2),
        )
        .unwrap()
    }
}
