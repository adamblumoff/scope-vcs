use crate::{
    auth::{
        runner::{attempt_token_hash, require_attempt, require_runner},
        tokens::generate_attempt_token,
    },
    error::ApiError,
    persistence::unix_now,
    persistence_ids::generate_prefixed_id,
    state::AppState,
};
use axum::{
    Json,
    extract::{Path, State},
    http::{
        HeaderMap, HeaderValue,
        header::{CONTENT_TYPE, HeaderName},
    },
    response::IntoResponse,
};
use scope_api_contract::{
    AppendAttemptLogRequest, AttemptCacheFinalizationOutcome, AttemptCacheFinalizationRequest,
    AttemptConclusionRequest, AttemptHeartbeatRequest, AttemptRecoveryStatusResponse,
    AttemptStatusResponse, AttemptStepStatusResponse, ClaimRunResponse, CompleteAttemptRequest,
    CompleteAttemptStepRequest, PinAttemptContainerImageRequest, PinAttemptContainerImageResponse,
    RunJobResponse, RunnerPollResponse, RunnerRunOffer, StepConclusionRequest,
};
use scope_domain::runs::run::{
    AttemptConclusion, PinnedContainerImage, RunAttemptStep, RunLogChunk, StepConclusion,
};
use scope_object_store::source_blob_bytes_bounded;
use std::time::Duration;

const ATTEMPT_LEASE_SECONDS: u64 = 90;
const MAX_SOURCE_BUNDLE_BYTES: usize = 128 * 1024 * 1024;
const POLL_ITERATIONS: usize = 20;

pub(crate) async fn poll(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<RunnerPollResponse>, ApiError> {
    let runner = require_runner(&state, &headers).await?;
    for iteration in 0..POLL_ITERATIONS {
        if let Some(run) = state
            .metadata
            .runs()
            .next_dispatchable_run(&runner.id)
            .await?
        {
            return Ok(Json(RunnerPollResponse {
                run: Some(RunnerRunOffer {
                    run_id: run.id,
                    repository_id: run.workflow.repository_id().to_string(),
                    workflow_name: run.workflow.path().name().to_string(),
                    git_oid: run.source.git_oid().to_string(),
                }),
            }));
        }
        if iteration + 1 < POLL_ITERATIONS {
            tokio::time::sleep(Duration::from_secs(1)).await;
        }
    }
    Ok(Json(RunnerPollResponse { run: None }))
}

pub(crate) async fn claim(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(run_id): Path<String>,
) -> Result<Json<ClaimRunResponse>, ApiError> {
    let runner = require_runner(&state, &headers).await?;
    let now = unix_now()?;
    let lease_expires_at_unix = now + ATTEMPT_LEASE_SECONDS;
    let (attempt_token, attempt_token_hash) = generate_attempt_token()?;
    let claim = state
        .metadata
        .runs()
        .claim_run(
            &run_id,
            &runner.id,
            &generate_prefixed_id("attempt_")?,
            &attempt_token_hash,
            now,
            lease_expires_at_unix,
        )
        .await?;
    Ok(Json(ClaimRunResponse {
        attempt_id: claim.attempt.id,
        attempt_token,
        lease_expires_at_unix,
        canary_phase: claim.canary_phase,
        job: RunJobResponse {
            run_id: claim.run.id,
            repository_id: claim.run.workflow.repository_id().to_string(),
            git_oid: claim.run.source.git_oid().to_string(),
            source_digest: claim.run.source.digest().to_string(),
            pinned_container_image: claim
                .run
                .pinned_container_image
                .as_ref()
                .map(|image| image.as_str().to_string()),
            workflow: claim.workflow_revision.definition().clone(),
        },
    }))
}

pub(crate) async fn pin_container_image(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(attempt_id): Path<String>,
    Json(input): Json<PinAttemptContainerImageRequest>,
) -> Result<Json<PinAttemptContainerImageResponse>, ApiError> {
    let token_hash = attempt_token_hash(&headers)?;
    let authenticated = require_attempt(&state, &headers, &attempt_id).await?;
    let claim = state
        .metadata
        .runs()
        .pin_attempt_container_image(
            &attempt_id,
            &authenticated.attempt.runner_id,
            &token_hash,
            PinnedContainerImage::parse(input.image)?,
            unix_now()?,
        )
        .await?;
    let image = claim
        .run
        .pinned_container_image
        .expect("successful image pin must persist an immutable image");
    Ok(Json(PinAttemptContainerImageResponse {
        image: image.as_str().to_string(),
    }))
}

pub(crate) async fn start_step(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((attempt_id, step_index)): Path<(String, u32)>,
) -> Result<Json<AttemptStatusResponse>, ApiError> {
    let token_hash = attempt_token_hash(&headers)?;
    let authenticated = require_attempt(&state, &headers, &attempt_id).await?;
    let claim = state
        .metadata
        .runs()
        .start_attempt_step(
            &attempt_id,
            &authenticated.attempt.runner_id,
            &token_hash,
            step_index,
            unix_now()?,
        )
        .await?;
    Ok(Json(attempt_status(&claim)))
}

pub(crate) async fn heartbeat(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(attempt_id): Path<String>,
    Json(_input): Json<AttemptHeartbeatRequest>,
) -> Result<Json<AttemptStatusResponse>, ApiError> {
    let token_hash = attempt_token_hash(&headers)?;
    let authenticated = require_attempt(&state, &headers, &attempt_id).await?;
    let now = unix_now()?;
    state
        .metadata
        .runs()
        .heartbeat_attempt(
            &attempt_id,
            &authenticated.attempt.runner_id,
            &token_hash,
            now,
            now + ATTEMPT_LEASE_SECONDS,
        )
        .await?;
    let claim = state
        .metadata
        .runs()
        .authenticate_attempt(&attempt_id, &token_hash, now)
        .await?;
    Ok(Json(attempt_status(&claim)))
}

pub(crate) async fn finalize_cache(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(attempt_id): Path<String>,
    Json(input): Json<AttemptCacheFinalizationRequest>,
) -> Result<axum::http::StatusCode, ApiError> {
    let token_hash = attempt_token_hash(&headers)?;
    let succeeded = matches!(input.outcome, AttemptCacheFinalizationOutcome::Succeeded);
    state
        .metadata
        .runs()
        .finalize_runner_protocol_canary_cache(&attempt_id, &token_hash, succeeded, unix_now()?)
        .await?;
    Ok(axum::http::StatusCode::NO_CONTENT)
}

pub(crate) async fn recovery_status(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(attempt_id): Path<String>,
) -> Result<Json<AttemptRecoveryStatusResponse>, ApiError> {
    let claim = require_attempt(&state, &headers, &attempt_id).await?;
    Ok(Json(AttemptRecoveryStatusResponse {
        next_log_sequence: state
            .metadata
            .runs()
            .next_attempt_log_sequence(&attempt_id)
            .await?,
        steps: claim.steps.iter().map(attempt_step_status).collect(),
    }))
}

pub(crate) async fn source(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(attempt_id): Path<String>,
) -> Result<impl IntoResponse, ApiError> {
    let claim = require_attempt(&state, &headers, &attempt_id).await?;
    let source = claim.run.source.snapshot();
    let bytes =
        source_blob_bytes_bounded(state.object_store.as_ref(), source, MAX_SOURCE_BUNDLE_BYTES)?;
    let mut response_headers = HeaderMap::new();
    response_headers.insert(
        CONTENT_TYPE,
        HeaderValue::from_static("application/octet-stream"),
    );
    response_headers.insert(
        HeaderName::from_static("x-scope-source-sha256"),
        HeaderValue::from_str(&source.sha256)
            .map_err(|_| ApiError::internal_message("source digest is not a valid header value"))?,
    );
    Ok((response_headers, bytes))
}

pub(crate) async fn append_log(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(attempt_id): Path<String>,
    Json(input): Json<AppendAttemptLogRequest>,
) -> Result<Json<scope_api_contract::RunLogResponse>, ApiError> {
    let token_hash = attempt_token_hash(&headers)?;
    let log = state
        .metadata
        .runs()
        .append_attempt_log(
            RunLogChunk::new(
                attempt_id,
                input.step_index,
                input.sequence,
                input.text,
                unix_now()?,
            )?,
            &token_hash,
            unix_now()?,
        )
        .await?;
    Ok(Json(scope_api_contract::RunLogResponse {
        attempt_id: log.chunk.attempt_id,
        step_index: log.chunk.step_index,
        position: log.position,
        sequence: log.chunk.sequence,
        text: log.chunk.text,
        created_at_unix: log.chunk.created_at_unix,
    }))
}

pub(crate) async fn complete_step(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((attempt_id, step_index)): Path<(String, u32)>,
    Json(input): Json<CompleteAttemptStepRequest>,
) -> Result<Json<AttemptStatusResponse>, ApiError> {
    let token_hash = attempt_token_hash(&headers)?;
    let authenticated = require_attempt(&state, &headers, &attempt_id).await?;
    let conclusion = match input.conclusion {
        StepConclusionRequest::Succeeded => StepConclusion::Succeeded,
        StepConclusionRequest::Failed { exit_code } => StepConclusion::Failed { exit_code },
    };
    let claim = state
        .metadata
        .runs()
        .complete_attempt_step(
            &attempt_id,
            &authenticated.attempt.runner_id,
            &token_hash,
            step_index,
            conclusion,
            unix_now()?,
        )
        .await?;
    Ok(Json(attempt_status(&claim)))
}

pub(crate) async fn complete(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(attempt_id): Path<String>,
    Json(input): Json<CompleteAttemptRequest>,
) -> Result<Json<AttemptStatusResponse>, ApiError> {
    let token_hash = attempt_token_hash(&headers)?;
    let authenticated = require_attempt(&state, &headers, &attempt_id).await?;
    let conclusion = match input.conclusion {
        AttemptConclusionRequest::SetupFailed { exit_code, message } => {
            AttemptConclusion::SetupFailed { exit_code, message }
        }
        AttemptConclusionRequest::TimedOut => AttemptConclusion::TimedOut,
        AttemptConclusionRequest::Canceled => AttemptConclusion::Canceled,
    };
    let claim = state
        .metadata
        .runs()
        .complete_attempt(
            &attempt_id,
            &authenticated.attempt.runner_id,
            &token_hash,
            conclusion,
            unix_now()?,
        )
        .await?;
    Ok(Json(attempt_status(&claim)))
}

pub(crate) async fn abandon(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(attempt_id): Path<String>,
) -> Result<Json<AttemptStatusResponse>, ApiError> {
    let token_hash = attempt_token_hash(&headers)?;
    let authenticated = require_attempt(&state, &headers, &attempt_id).await?;
    let claim = state
        .metadata
        .runs()
        .abandon_attempt(
            &attempt_id,
            &authenticated.attempt.runner_id,
            &token_hash,
            unix_now()?,
        )
        .await?;
    Ok(Json(attempt_status(&claim)))
}

fn attempt_status(claim: &scope_postgres::db::DispatchClaim) -> AttemptStatusResponse {
    AttemptStatusResponse {
        state: claim.attempt.state,
        cancellation_requested: claim.run.cancellation_requested,
        lease_expires_at_unix: claim.attempt.lease_expires_at_unix,
    }
}

fn attempt_step_status(step: &RunAttemptStep) -> AttemptStepStatusResponse {
    AttemptStepStatusResponse {
        step_index: step.step_index,
        state: step.state,
        started_at_unix: step.started_at_unix,
        completed_at_unix: step.completed_at_unix,
        exit_code: step.exit_code,
    }
}
