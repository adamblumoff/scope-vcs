use crate::{
    auth::{
        runtime::{attempt_token_hash, bootstrap_token_hash, require_attempt},
        tokens::generate_attempt_token,
    },
    error::ApiError,
    git::run_source::materialize_run_source_bundle,
    persistence::unix_now,
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
    AppendAttemptLogRequest, AttemptConclusionRequest, AttemptHeartbeatRequest,
    AttemptHeartbeatResponse, AttemptRecoveryStatusResponse, AttemptStatusResponse,
    AttemptStepStatusResponse, ClaimRuntimeResponse, CompleteAttemptRequest,
    CompleteAttemptStepRequest, ReportAttemptCacheFinalizationsRequest,
    ReportAttemptCachePreparationsRequest, RunChangeKind, RunJobResponse, StepConclusionRequest,
};
use scope_domain::runs::cache::{CacheIdentity, CacheNamespace, CachePlatform};
use scope_domain::runs::run::{AttemptConclusion, RunAttemptStep, RunLogChunk, StepConclusion};

const ATTEMPT_LEASE_SECONDS: u64 = 90;
const MAX_SOURCE_BUNDLE_BYTES: usize = 128 * 1024 * 1024;
pub(crate) async fn claim(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(attempt_id): Path<String>,
) -> Result<Json<ClaimRuntimeResponse>, ApiError> {
    let bootstrap_hash = bootstrap_token_hash(&headers)?;
    let now = unix_now()?;
    let lease_expires_at_unix = now + ATTEMPT_LEASE_SECONDS;
    let (attempt_token, attempt_token_hash) = generate_attempt_token()?;
    let claim = state
        .metadata
        .runs()
        .claim_runtime(
            &attempt_id,
            &bootstrap_hash,
            &attempt_token_hash,
            now,
            lease_expires_at_unix,
        )
        .await?;
    publish_claim_status_change(&state, &claim).await;
    let job_definition = claimed_job_definition(&claim);
    let cache_grant = issue_cache_grant(&state, &claim)?;
    Ok(Json(ClaimRuntimeResponse {
        attempt_token,
        lease_expires_at_unix,
        cache_endpoint: state.cache_grants.endpoint().to_string(),
        cache_grant,
        job: RunJobResponse {
            run_id: claim.run.id,
            job_key: claim.job.key.as_str().to_string(),
            repository_id: claim.run.workflow.repository_id().to_string(),
            workflow_path: claim.run.workflow.path().as_str().to_string(),
            git_oid: claim.run.source.git_oid().to_string(),
            source_digest: claim.run.source.source_identity().to_string(),
            pinned_container_image: claim.job.pinned_container_image.as_str().to_string(),
            definition: job_definition,
        },
    }))
}

pub(crate) async fn start_step(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((attempt_id, step_index)): Path<(String, u32)>,
) -> Result<Json<AttemptStatusResponse>, ApiError> {
    let token_hash = attempt_token_hash(&headers)?;
    let claim = state
        .metadata
        .runs()
        .start_attempt_step(&attempt_id, &token_hash, step_index, unix_now()?)
        .await?;
    publish_claim_status_change(&state, &claim).await;
    Ok(Json(attempt_status(&claim)))
}

pub(crate) async fn heartbeat(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(attempt_id): Path<String>,
    Json(_input): Json<AttemptHeartbeatRequest>,
) -> Result<Json<AttemptHeartbeatResponse>, ApiError> {
    let token_hash = attempt_token_hash(&headers)?;
    let now = unix_now()?;
    state
        .metadata
        .runs()
        .heartbeat_attempt(&attempt_id, &token_hash, now, now + ATTEMPT_LEASE_SECONDS)
        .await?;
    let claim = state
        .metadata
        .runs()
        .authenticate_attempt(&attempt_id, &token_hash, now)
        .await?;
    let cache_grant = issue_cache_grant(&state, &claim)?;
    Ok(Json(AttemptHeartbeatResponse {
        status: attempt_status(&claim),
        cache_grant,
    }))
}

pub(crate) async fn report_cache_preparations(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(attempt_id): Path<String>,
    Json(input): Json<ReportAttemptCachePreparationsRequest>,
) -> Result<axum::http::StatusCode, ApiError> {
    let token_hash = attempt_token_hash(&headers)?;
    let changed = state
        .metadata
        .runs()
        .report_attempt_cache_preparations(
            &attempt_id,
            &token_hash,
            input
                .caches
                .into_iter()
                .map(|cache| scope_postgres::db::AttemptCachePreparationCommand {
                    cache_name: cache.cache_name,
                    identity_digest: cache.identity_digest,
                    preparation: cache.preparation,
                    prepare_ms: cache.prepare_ms,
                })
                .collect(),
            unix_now()?,
        )
        .await?;
    if let Some(claim) = changed {
        publish_claim_status_change(&state, &claim).await;
    }
    Ok(axum::http::StatusCode::NO_CONTENT)
}

pub(crate) async fn report_cache_finalizations(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(attempt_id): Path<String>,
    Json(input): Json<ReportAttemptCacheFinalizationsRequest>,
) -> Result<axum::http::StatusCode, ApiError> {
    let token_hash = attempt_token_hash(&headers)?;
    let changed = state
        .metadata
        .runs()
        .report_attempt_cache_finalizations(
            &attempt_id,
            &token_hash,
            input
                .caches
                .into_iter()
                .map(
                    |cache| scope_postgres::db::AttemptCacheFinalizationCommand {
                        identity_digest: cache.identity_digest,
                        final_state: cache.final_state,
                        finalize_ms: cache.finalize_ms,
                    },
                )
                .collect(),
            unix_now()?,
        )
        .await?;
    if let Some(claim) = changed {
        publish_claim_status_change(&state, &claim).await;
    }
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
    let source_identity = claim.run.source.source_identity().to_string();
    let source = claim.run.source.clone();
    let source_state = state.clone();
    let _materialization_permit = source
        .logical_git_head()
        .is_some()
        .then(|| state.runtime_budgets.try_git_materialization())
        .transpose()?;
    let materialized = tokio::task::spawn_blocking(move || {
        materialize_run_source_bundle(&source_state, &source, MAX_SOURCE_BUNDLE_BYTES)
    })
    .await
    .map_err(|error| {
        ApiError::internal_message(format!("run source materialization failed: {error}"))
    })??;
    let mut response_headers = HeaderMap::new();
    response_headers.insert(
        CONTENT_TYPE,
        HeaderValue::from_static("application/octet-stream"),
    );
    response_headers.insert(
        HeaderName::from_static("x-scope-source-sha256"),
        HeaderValue::from_str(&materialized.sha256)
            .map_err(|_| ApiError::internal_message("source digest is not a valid header value"))?,
    );
    response_headers.insert(
        HeaderName::from_static("x-scope-source-identity"),
        HeaderValue::from_str(&source_identity).map_err(|_| {
            ApiError::internal_message("source identity is not a valid header value")
        })?,
    );
    Ok((response_headers, materialized.bytes))
}

pub(crate) async fn append_log(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(attempt_id): Path<String>,
    Json(input): Json<AppendAttemptLogRequest>,
) -> Result<Json<scope_api_contract::RunLogResponse>, ApiError> {
    let token_hash = attempt_token_hash(&headers)?;
    let appended = state
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
    if appended.appended {
        state
            .publish_run_change(
                &appended.repo_id,
                appended.log.run_id.clone(),
                RunChangeKind::LogsAppended,
            )
            .await;
    }
    let log = appended.log;
    Ok(Json(scope_api_contract::RunLogResponse {
        attempt_id: log.chunk.attempt_id,
        job_key: log.job_key,
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
    let conclusion = match input.conclusion {
        StepConclusionRequest::Succeeded => StepConclusion::Succeeded,
        StepConclusionRequest::Failed { exit_code } => StepConclusion::Failed { exit_code },
    };
    let claim = state
        .metadata
        .runs()
        .complete_attempt_step(
            &attempt_id,
            &token_hash,
            step_index,
            conclusion,
            input.logs_truncated,
            unix_now()?,
        )
        .await?;
    publish_claim_status_change(&state, &claim).await;
    Ok(Json(attempt_status(&claim)))
}

pub(crate) async fn complete(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(attempt_id): Path<String>,
    Json(input): Json<CompleteAttemptRequest>,
) -> Result<Json<AttemptStatusResponse>, ApiError> {
    let token_hash = attempt_token_hash(&headers)?;
    let conclusion = match input.conclusion {
        AttemptConclusionRequest::Succeeded => AttemptConclusion::Succeeded,
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
            &token_hash,
            conclusion,
            input.logs_truncated,
            unix_now()?,
        )
        .await?;
    publish_claim_status_change(&state, &claim).await;
    Ok(Json(attempt_status(&claim)))
}

pub(crate) async fn abandon(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(attempt_id): Path<String>,
) -> Result<Json<AttemptStatusResponse>, ApiError> {
    let token_hash = attempt_token_hash(&headers)?;
    let claim = state
        .metadata
        .runs()
        .abandon_attempt(&attempt_id, &token_hash, unix_now()?)
        .await?;
    publish_claim_status_change(&state, &claim).await;
    Ok(Json(attempt_status(&claim)))
}

async fn publish_claim_status_change(state: &AppState, claim: &scope_postgres::db::DispatchClaim) {
    state
        .publish_run_change(
            claim.run.workflow.repository_id(),
            claim.run.id.clone(),
            RunChangeKind::StatusChanged,
        )
        .await;
}

fn attempt_status(claim: &scope_postgres::db::DispatchClaim) -> AttemptStatusResponse {
    AttemptStatusResponse {
        state: claim.attempt.state,
        cancellation_requested: claim.run.cancellation_requested,
        lease_expires_at_unix: claim.attempt.lease_expires_at_unix,
    }
}

fn claimed_job_definition(
    claim: &scope_postgres::db::DispatchClaim,
) -> scope_domain::runs::workflow::WorkflowJob {
    claim
        .workflow_revision
        .definition()
        .job(&claim.job.key)
        .expect("claimed run job definition must exist")
        .clone()
}

fn issue_cache_grant(
    state: &AppState,
    claim: &scope_postgres::db::DispatchClaim,
) -> Result<String, ApiError> {
    let job_definition = claimed_job_definition(claim);
    let namespace = CacheNamespace::workflow(claim.run.workflow.path(), &claim.job.key);
    let allowed_identity_digests = job_definition
        .caches()
        .iter()
        .map(|cache| {
            let digest = CacheIdentity::new(
                claim.run.workflow.repository_id(),
                namespace.clone(),
                cache.clone(),
                &claim.job.pinned_container_image,
                CachePlatform::LinuxAmd64,
            )?
            .digest();
            scope_cache_domain::CacheDigest::parse(digest).map_err(ApiError::bad_request)
        })
        .collect::<Result<Vec<_>, ApiError>>()?;
    state
        .cache_grants
        .issue(
            claim.attempt.id.clone(),
            scope_cache_domain::RepositoryId::parse(claim.run.workflow.repository_id().to_string())
                .map_err(ApiError::bad_request)?,
            allowed_identity_digests,
            claim.attempt.lease_expires_at_unix,
        )
        .map_err(|error| ApiError::internal_message(error.to_string()))
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
