use crate::error::ApiError;
use scope_api_contract::{
    RepositoryRunState, RepositoryRunSummaryResponse, RepositoryRunTrigger, RunResponse,
};
use scope_domain::runs::{
    job::{RunJob, can_retry_run},
    run::{Run, RunState},
    source::RunTrigger,
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
        state: run_state(run.state),
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
        trigger: repository_run_trigger(run.trigger),
        state: repository_run_state(run.state),
        cancellation_requested: run.cancellation_requested,
        created_at_unix: run.created_at_unix,
        updated_at_unix: run.updated_at_unix,
        completed_at_unix: run.completed_at_unix,
        can_cancel: run.can_request_cancellation(),
        can_retry: can_retry_run(run, jobs),
    })
}

pub(super) fn run_state(state: RunState) -> scope_api_contract::RunState {
    match state {
        RunState::Queued => scope_api_contract::RunState::Queued,
        RunState::Dispatching => scope_api_contract::RunState::Dispatching,
        RunState::Running => scope_api_contract::RunState::Running,
        RunState::Succeeded => scope_api_contract::RunState::Succeeded,
        RunState::Failed => scope_api_contract::RunState::Failed,
        RunState::Canceled => scope_api_contract::RunState::Canceled,
        RunState::Lost => scope_api_contract::RunState::Lost,
    }
}

fn repository_run_state(state: RunState) -> RepositoryRunState {
    match state {
        RunState::Queued => RepositoryRunState::Queued,
        RunState::Dispatching => RepositoryRunState::Dispatching,
        RunState::Running => RepositoryRunState::Running,
        RunState::Succeeded => RepositoryRunState::Succeeded,
        RunState::Failed => RepositoryRunState::Failed,
        RunState::Canceled => RepositoryRunState::Canceled,
        RunState::Lost => RepositoryRunState::Lost,
    }
}

fn repository_run_trigger(trigger: RunTrigger) -> RepositoryRunTrigger {
    match trigger {
        RunTrigger::Manual => RepositoryRunTrigger::Manual,
        RunTrigger::PushMain => RepositoryRunTrigger::PushMain,
    }
}
