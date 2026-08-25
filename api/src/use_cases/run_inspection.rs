use crate::{error::ApiError, repo_access::find_repo, state::AppState};
use scope_domain::{
    repo_actions::ensure_repo_member,
    repository::Repository,
    runs::{job::RunJob, run::Run},
};
use scope_postgres::db::{RunDetail, StoredRunLog};

pub(crate) struct RunStepLogs {
    pub(crate) logs: Vec<StoredRunLog>,
    pub(crate) next_after: u64,
    pub(crate) logs_truncated: bool,
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
) -> Result<Repository, ApiError> {
    let repo = find_repo(state, owner, name).await?;
    ensure_repo_member(&repo, user_id).map_err(ApiError::from)?;
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
    after: u64,
    limit: u64,
) -> Result<RunStepLogs, ApiError> {
    require_run_access(state, user_id, owner, repo_name, run_id).await?;
    let page = state
        .metadata
        .runs()
        .attempt_step_logs_after(run_id, attempt_id, step_index, after, limit)
        .await?;
    let next_after = page.logs.last().map_or(after, |stored| stored.position);
    Ok(RunStepLogs {
        logs: page.logs,
        next_after,
        logs_truncated: page.logs_truncated,
    })
}
