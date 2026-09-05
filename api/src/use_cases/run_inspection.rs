use crate::{error::ApiError, state::AppState};
use scope_domain::{
    repository::access::RepositoryAccessContext,
    runs::{job::RunJob, run::Run},
};
use scope_postgres::db::{RunDetail, StepLogCursor, StoredRunLog};

pub(crate) struct RunStepLogs {
    pub(crate) logs: Vec<StoredRunLog>,
    pub(crate) next_after: u64,
    pub(crate) logs_truncated: bool,
    pub(crate) has_earlier: bool,
    pub(crate) has_more: bool,
}

pub(crate) struct InspectedRun {
    pub(crate) run: Run,
    pub(crate) jobs: Vec<RunJob>,
    pub(crate) logs_truncated: bool,
}

pub(crate) async fn require_repo_member(
    state: &AppState,
    user_id: &str,
    owner: &str,
    name: &str,
) -> Result<RepositoryAccessContext, ApiError> {
    let repo = state
        .metadata
        .repositories()
        .repository_access(owner, name, Some(user_id))
        .await?
        .ok_or_else(|| ApiError::not_found(format!("repo {owner}/{name} not found")))?;
    repo.ensure_member()?;
    Ok(repo)
}

pub(crate) async fn require_run_access(
    state: &AppState,
    user_id: &str,
    owner: &str,
    repo_name: &str,
    run_id: &str,
) -> Result<Run, ApiError> {
    let repo = require_repo_member(state, user_id, owner, repo_name).await?;
    let run = state
        .metadata
        .runs()
        .run(run_id)
        .await?
        .ok_or_else(|| ApiError::not_found("run not found"))?;
    if !run.belongs_to_repository(&repo.record.id) {
        return Err(ApiError::not_found("run not found"));
    }
    Ok(run)
}

pub(crate) async fn inspect_run(
    state: &AppState,
    user_id: &str,
    owner: &str,
    repo_name: &str,
    run_id: &str,
) -> Result<InspectedRun, ApiError> {
    require_run_access(state, user_id, owner, repo_name, run_id).await?;
    let snapshot = state
        .metadata
        .runs()
        .run_snapshot(run_id)
        .await?
        .ok_or_else(|| ApiError::not_found("run not found"))?;
    Ok(InspectedRun {
        run: snapshot.run,
        jobs: snapshot.jobs,
        logs_truncated: snapshot.logs_truncated,
    })
}

pub(crate) async fn inspect_run_detail(
    state: &AppState,
    user_id: &str,
    owner: &str,
    repo_name: &str,
    run_id: &str,
) -> Result<RunDetail, ApiError> {
    require_run_access(state, user_id, owner, repo_name, run_id).await?;
    state
        .metadata
        .runs()
        .run_detail(run_id)
        .await?
        .ok_or_else(|| ApiError::not_found("run not found"))
}

#[allow(clippy::too_many_arguments)]
pub(crate) async fn inspect_run_step_logs(
    state: &AppState,
    user_id: &str,
    owner: &str,
    repo_name: &str,
    run_id: &str,
    attempt_id: &str,
    step_index: u32,
    cursor: StepLogCursor,
    limit: u64,
) -> Result<RunStepLogs, ApiError> {
    require_run_access(state, user_id, owner, repo_name, run_id).await?;
    let page = state
        .metadata
        .runs()
        .attempt_step_logs(run_id, attempt_id, step_index, cursor, limit)
        .await?;
    let next_after = page.logs.last().map_or_else(
        || match cursor {
            StepLogCursor::After(position) => position,
            _ => 0,
        },
        |stored| stored.position,
    );
    Ok(RunStepLogs {
        logs: page.logs,
        next_after,
        logs_truncated: page.logs_truncated,
        has_earlier: page.has_earlier,
        has_more: page.has_more,
    })
}
