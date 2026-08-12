use crate::{error::ApiError, http::responses::RepositoryRunSummaryResponse};
use scope_api_contract::RunResponse;
use scope_domain::runs::{
    job::{RunJob, can_retry_run},
    run::Run,
};

pub(super) fn run_response(
    run: &Run,
    _jobs: &[RunJob],
    logs_truncated: bool,
) -> Result<RunResponse, ApiError> {
    Ok(RunResponse {
        id: run.id.clone(),
        repository_id: run.workflow.repository_id().to_string(),
        workflow_name: run.workflow.path().name().to_string(),
        git_oid: run.source.git_oid().to_string(),
        state: run.state,
        cancellation_requested: run.cancellation_requested,
        logs_truncated,
        created_at_unix: run.created_at_unix,
        updated_at_unix: run.updated_at_unix,
        completed_at_unix: run.completed_at_unix,
    })
}

pub(super) fn repository_run_summary(
    run: &Run,
    jobs: &[RunJob],
) -> Result<RepositoryRunSummaryResponse, ApiError> {
    Ok(RepositoryRunSummaryResponse {
        id: run.id.clone(),
        workflow_name: run.workflow.path().name().to_string(),
        git_oid: run.source.git_oid().to_string(),
        state: run.state.into(),
        cancellation_requested: run.cancellation_requested,
        created_at_unix: run.created_at_unix,
        updated_at_unix: run.updated_at_unix,
        completed_at_unix: run.completed_at_unix,
        can_cancel: run.can_request_cancellation(),
        can_retry: can_retry_run(run, jobs),
    })
}
