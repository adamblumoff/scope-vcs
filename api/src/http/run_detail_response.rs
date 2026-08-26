use crate::error::ApiError;
use scope_api_contract::{
    RepositoryRunAttemptResponse, RepositoryRunAttemptState, RepositoryRunCacheColdReason,
    RepositoryRunCacheFinalState, RepositoryRunCacheObservationResponse,
    RepositoryRunCachePreparation, RepositoryRunCacheResponse,
    RepositoryRunCacheSetupObservationResponse, RepositoryRunDetailResponse,
    RepositoryRunJobDetailResponse, RepositoryRunJobResponse, RepositoryRunJobState,
    RepositoryRunStepResponse, RepositoryRunStepState, RepositoryRunSummaryResponse,
    RepositoryRunTerminalReason,
};
use scope_domain::runs::{
    attempt::AttemptState,
    cache::{
        definition::WorkflowCache,
        observation::{
            AttemptCacheObservation, CacheColdReason, CacheFinalState, CachePreparation,
        },
    },
    job::RunJobState,
    step::{AttemptTerminalReason, StepState},
};
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
        let workflow_job = workflow.job(&attempt.job_key).ok_or_else(|| {
            ApiError::internal_message(
                "persisted run attempt job is missing from its workflow revision",
            )
        })?;
        let caches = cache_responses(
            workflow_job.caches(),
            attempt_detail.caches,
            &attempt.id,
            detail.workflow_revision.workflow().path().as_str(),
            &job_key,
        )?;
        let workflow_steps = workflow_job.steps();
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
                    state: step_state(step.state),
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
                number: attempt.number,
                external_run_id: attempt.external_run_id,
                runtime_version: attempt.runtime_version,
                state: attempt_state(attempt.state),
                created_at_unix: attempt.created_at_unix,
                started_at_unix: attempt.started_at_unix,
                completed_at_unix: attempt.completed_at_unix,
                terminal_reason: attempt.terminal_reason.map(terminal_reason),
                cache_setup: attempt_detail.cache_setup.map(|setup| {
                    RepositoryRunCacheSetupObservationResponse {
                        authorization_ms: setup.authorization_ms,
                        wall_ms: setup.wall_ms,
                    }
                }),
                caches,
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
            let attempts = attempts_by_job.remove(key).unwrap_or_default();
            Ok(RepositoryRunJobDetailResponse {
                job: RepositoryRunJobResponse {
                    key: key.to_string(),
                    needs: definition
                        .needs()
                        .iter()
                        .map(|dependency| dependency.as_str().to_string())
                        .collect(),
                    pinned_container_image: job.pinned_container_image.as_str().to_string(),
                    state: job_state(job.state),
                    created_at_unix: job.created_at_unix,
                    started_at_unix: attempts
                        .iter()
                        .filter_map(|attempt| attempt.started_at_unix)
                        .min(),
                    updated_at_unix: job.updated_at_unix,
                    completed_at_unix: job.completed_at_unix,
                },
                attempts,
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

fn cache_responses(
    definitions: &[WorkflowCache],
    observations: Vec<AttemptCacheObservation>,
    attempt_id: &str,
    workflow_path: &str,
    job_key: &str,
) -> Result<Vec<RepositoryRunCacheResponse>, ApiError> {
    let mut observations_by_name = BTreeMap::new();
    for observation in observations {
        if observation.attempt_id != attempt_id
            || observation.workflow_path.as_str() != workflow_path
            || observation.job_key.as_str() != job_key
        {
            return Err(ApiError::internal_message(
                "persisted cache observation does not match its run attempt",
            ));
        }
        if observations_by_name
            .insert(observation.cache_name.clone(), observation)
            .is_some()
        {
            return Err(ApiError::internal_message(
                "persisted run attempt contains duplicate cache observations",
            ));
        }
    }
    let caches = definitions
        .iter()
        .map(|definition| {
            let observation = observations_by_name
                .remove(definition.as_str())
                .map(|observation| RepositoryRunCacheObservationResponse {
                    workflow_path: observation.workflow_path.as_str().to_string(),
                    job_key: observation.job_key.as_str().to_string(),
                    identity_digest: observation.identity_digest,
                    preparation: cache_preparation(observation.preparation),
                    key_ms: observation.timing.key_ms,
                    metadata_ms: observation.timing.metadata_ms,
                    size_bytes: observation.timing.size_bytes,
                    download_verify_ms: observation.timing.download_verify_ms,
                    sync_ms: observation.timing.sync_ms,
                    extraction_ms: observation.timing.extraction_ms,
                    prepare_ms: observation.timing.prepare_ms,
                    final_state: cache_final_state(observation.final_state),
                    finalize_ms: observation.finalize_ms,
                });
            RepositoryRunCacheResponse {
                name: definition.as_str().to_string(),
                path: definition.mount_path().to_string(),
                observation,
            }
        })
        .collect();
    if !observations_by_name.is_empty() {
        return Err(ApiError::internal_message(
            "persisted cache observation is missing from its workflow revision",
        ));
    }
    Ok(caches)
}

fn job_state(state: RunJobState) -> RepositoryRunJobState {
    match state {
        RunJobState::Blocked => RepositoryRunJobState::Blocked,
        RunJobState::Queued => RepositoryRunJobState::Queued,
        RunJobState::Dispatching => RepositoryRunJobState::Dispatching,
        RunJobState::Running => RepositoryRunJobState::Running,
        RunJobState::Succeeded => RepositoryRunJobState::Succeeded,
        RunJobState::Failed => RepositoryRunJobState::Failed,
        RunJobState::Skipped => RepositoryRunJobState::Skipped,
        RunJobState::Canceled => RepositoryRunJobState::Canceled,
        RunJobState::Lost => RepositoryRunJobState::Lost,
    }
}

fn attempt_state(state: AttemptState) -> RepositoryRunAttemptState {
    match state {
        AttemptState::Dispatching => RepositoryRunAttemptState::Dispatching,
        AttemptState::Running => RepositoryRunAttemptState::Running,
        AttemptState::Succeeded => RepositoryRunAttemptState::Succeeded,
        AttemptState::Failed => RepositoryRunAttemptState::Failed,
        AttemptState::Canceled => RepositoryRunAttemptState::Canceled,
        AttemptState::Lost => RepositoryRunAttemptState::Lost,
    }
}

fn step_state(state: StepState) -> RepositoryRunStepState {
    match state {
        StepState::Pending => RepositoryRunStepState::Pending,
        StepState::Running => RepositoryRunStepState::Running,
        StepState::Succeeded => RepositoryRunStepState::Succeeded,
        StepState::Failed => RepositoryRunStepState::Failed,
        StepState::Canceled => RepositoryRunStepState::Canceled,
        StepState::Lost => RepositoryRunStepState::Lost,
        StepState::Skipped => RepositoryRunStepState::Skipped,
    }
}

fn terminal_reason(reason: AttemptTerminalReason) -> RepositoryRunTerminalReason {
    match reason {
        AttemptTerminalReason::StepFailed {
            step_index,
            exit_code,
        } => RepositoryRunTerminalReason::StepFailed {
            step_index,
            exit_code,
        },
        AttemptTerminalReason::TimedOut { step_index } => {
            RepositoryRunTerminalReason::TimedOut { step_index }
        }
        AttemptTerminalReason::Canceled { step_index } => {
            RepositoryRunTerminalReason::Canceled { step_index }
        }
        AttemptTerminalReason::ExecutionLost { step_index } => {
            RepositoryRunTerminalReason::ExecutionLost { step_index }
        }
        AttemptTerminalReason::RuntimeSetupFailed { exit_code, message } => {
            RepositoryRunTerminalReason::RuntimeSetupFailed { exit_code, message }
        }
    }
}

fn cache_preparation(preparation: CachePreparation) -> RepositoryRunCachePreparation {
    match preparation {
        CachePreparation::Exact => RepositoryRunCachePreparation::Exact,
        CachePreparation::Compatible => RepositoryRunCachePreparation::Compatible,
        CachePreparation::Cold { reason } => RepositoryRunCachePreparation::Cold {
            reason: cache_cold_reason(reason),
        },
    }
}

fn cache_cold_reason(reason: CacheColdReason) -> RepositoryRunCacheColdReason {
    match reason {
        CacheColdReason::MetadataMissing => RepositoryRunCacheColdReason::MetadataMissing,
        CacheColdReason::MetadataInvalid => RepositoryRunCacheColdReason::MetadataInvalid,
        CacheColdReason::MetadataNotReady => RepositoryRunCacheColdReason::MetadataNotReady,
        CacheColdReason::VolumeMissing => RepositoryRunCacheColdReason::VolumeMissing,
        CacheColdReason::VolumeInvalid => RepositoryRunCacheColdReason::VolumeInvalid,
        CacheColdReason::BackingDirectoryMissing => {
            RepositoryRunCacheColdReason::BackingDirectoryMissing
        }
    }
}

fn cache_final_state(state: CacheFinalState) -> RepositoryRunCacheFinalState {
    match state {
        CacheFinalState::Pending => RepositoryRunCacheFinalState::Pending,
        CacheFinalState::Ready => RepositoryRunCacheFinalState::Ready,
        CacheFinalState::Evicted => RepositoryRunCacheFinalState::Evicted,
    }
}
