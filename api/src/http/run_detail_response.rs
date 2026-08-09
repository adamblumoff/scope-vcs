use crate::{
    error::ApiError,
    http::responses::{
        RepositoryRunAttemptResponse, RepositoryRunDetailResponse, RepositoryRunJobDetailResponse,
        RepositoryRunJobResponse, RepositoryRunStepResponse, RepositoryRunSummaryResponse,
    },
};
use scope_domain::runs::workflow::RunnerSelector;
use scope_postgres::db::RunDetail;
use std::collections::BTreeMap;

pub(super) fn build_run_detail_response(
    detail: RunDetail,
    run: RepositoryRunSummaryResponse,
) -> Result<RepositoryRunDetailResponse, ApiError> {
    let workflow = detail.workflow_revision.definition();
    let mut attempts_by_job = BTreeMap::<String, Vec<RepositoryRunAttemptResponse>>::new();
    for attempt_detail in detail.attempts {
        let attempt = attempt_detail.attempt;
        let job_key = attempt.job_key.as_str().to_string();
        let workflow_steps = workflow
            .job(&attempt.job_key)
            .ok_or_else(|| {
                ApiError::internal_message(
                    "persisted run attempt job is missing from its workflow revision",
                )
            })?
            .steps();
        let steps = attempt_detail
            .steps
            .into_iter()
            .map(|step| {
                let definition = workflow_steps
                    .get(step.step_index as usize)
                    .ok_or_else(|| {
                        ApiError::internal_message(
                            "persisted run step is missing from its workflow revision",
                        )
                    })?;
                Ok(RepositoryRunStepResponse {
                    index: step.step_index,
                    name: definition.name().to_string(),
                    command: definition.run().to_string(),
                    state: step.state.into(),
                    started_at_unix: step.started_at_unix,
                    completed_at_unix: step.completed_at_unix,
                    exit_code: step.exit_code,
                })
            })
            .collect::<Result<Vec<_>, ApiError>>()?;
        attempts_by_job
            .entry(job_key)
            .or_default()
            .push(RepositoryRunAttemptResponse {
                id: attempt.id,
                runner_id: attempt.runner_id,
                runner_name: attempt.runner_name,
                state: attempt.state.into(),
                created_at_unix: attempt.created_at_unix,
                started_at_unix: attempt.started_at_unix,
                completed_at_unix: attempt.completed_at_unix,
                terminal_reason: attempt.terminal_reason.map(Into::into),
                steps,
            });
    }

    let mut persisted_jobs = detail
        .jobs
        .into_iter()
        .map(|job| (job.key.as_str().to_string(), job))
        .collect::<BTreeMap<_, _>>();
    let jobs = workflow
        .jobs()
        .iter()
        .map(|definition| {
            let key = definition.id().as_str();
            let job = persisted_jobs.remove(key).ok_or_else(|| {
                ApiError::internal_message("persisted run job is missing from its workflow")
            })?;
            Ok(RepositoryRunJobDetailResponse {
                job: RepositoryRunJobResponse {
                    key: key.to_string(),
                    needs: definition
                        .needs()
                        .iter()
                        .map(|dependency| dependency.as_str().to_string())
                        .collect(),
                    desired_runner: match &job.desired_runner {
                        RunnerSelector::Any => None,
                        RunnerSelector::Named(name) => Some(name.clone()),
                    },
                    state: job.state.into(),
                    created_at_unix: job.created_at_unix,
                    updated_at_unix: job.updated_at_unix,
                    completed_at_unix: job.completed_at_unix,
                },
                attempts: attempts_by_job.remove(key).unwrap_or_default(),
            })
        })
        .collect::<Result<Vec<_>, ApiError>>()?;
    if !persisted_jobs.is_empty() || !attempts_by_job.is_empty() {
        return Err(ApiError::internal_message(
            "persisted run execution is not present in its workflow revision",
        ));
    }
    Ok(RepositoryRunDetailResponse { run, jobs })
}
