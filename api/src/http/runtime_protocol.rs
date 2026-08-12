use crate::{
    auth::{
        runtime::{attempt_token_hash, bootstrap_token_hash, require_attempt},
        tokens::generate_attempt_token,
    },
    error::ApiError,
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
    AttemptRecoveryStatusResponse, AttemptStatusResponse, AttemptStepStatusResponse,
    CacheDownloadSessionResponse, CacheUploadSessionResponse, ClaimRuntimeResponse,
    CommitCacheUploadRequest, CompleteAttemptRequest, CompleteAttemptStepRequest,
    ReportAttemptCacheFinalizationsRequest, ReportAttemptCachePreparationsRequest, RunJobResponse,
    StepConclusionRequest,
};
use scope_domain::runs::cache::{CacheIdentity, CacheNamespace, CachePlatform};
use scope_domain::runs::run::{AttemptConclusion, RunAttemptStep, RunLogChunk, StepConclusion};
use scope_object_store::source_blob_bytes_bounded;

const ATTEMPT_LEASE_SECONDS: u64 = 90;
const MAX_SOURCE_BUNDLE_BYTES: usize = 128 * 1024 * 1024;
const CACHE_URL_TTL_SECONDS: u32 = 15 * 60;
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
    Ok(Json(ClaimRuntimeResponse {
        attempt_token,
        lease_expires_at_unix,
        job: RunJobResponse {
            run_id: claim.run.id,
            job_key: claim.job.key.as_str().to_string(),
            repository_id: claim.run.workflow.repository_id().to_string(),
            workflow_path: claim.run.workflow.path().as_str().to_string(),
            git_oid: claim.run.source.git_oid().to_string(),
            source_digest: claim.run.source.digest().to_string(),
            pinned_container_image: claim.job.pinned_container_image.as_str().to_string(),
            definition: claim
                .workflow_revision
                .definition()
                .job(&claim.job.key)
                .expect("claimed run job definition must exist")
                .clone(),
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
    Ok(Json(attempt_status(&claim)))
}

pub(crate) async fn heartbeat(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(attempt_id): Path<String>,
    Json(_input): Json<AttemptHeartbeatRequest>,
) -> Result<Json<AttemptStatusResponse>, ApiError> {
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
    Ok(Json(attempt_status(&claim)))
}

pub(crate) async fn report_cache_preparations(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(attempt_id): Path<String>,
    Json(input): Json<ReportAttemptCachePreparationsRequest>,
) -> Result<axum::http::StatusCode, ApiError> {
    let token_hash = attempt_token_hash(&headers)?;
    state
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
    Ok(axum::http::StatusCode::NO_CONTENT)
}

pub(crate) async fn report_cache_finalizations(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(attempt_id): Path<String>,
    Json(input): Json<ReportAttemptCacheFinalizationsRequest>,
) -> Result<axum::http::StatusCode, ApiError> {
    let token_hash = attempt_token_hash(&headers)?;
    state
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
    Ok(axum::http::StatusCode::NO_CONTENT)
}

pub(crate) async fn cache_download(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((attempt_id, identity_digest)): Path<(String, String)>,
) -> Result<Json<CacheDownloadSessionResponse>, ApiError> {
    let claim = require_cache_attempt(&state, &headers, &attempt_id).await?;
    require_cache_identity(&claim, &identity_digest)?;
    let Some(object) = state
        .metadata
        .runs()
        .ready_cache_object(&identity_digest)
        .await?
    else {
        return Ok(Json(CacheDownloadSessionResponse {
            download_url: None,
            checksum_sha256: None,
            size_bytes: None,
        }));
    };
    let signer = state.cache_presigner.as_ref().ok_or_else(|| {
        ApiError::infrastructure_unavailable("remote run cache is not configured")
    })?;
    let download_url = signer
        .presign("GET", &object.object_key, CACHE_URL_TTL_SECONDS)
        .map_err(|error| ApiError::internal_message(error.to_string()))?;
    Ok(Json(CacheDownloadSessionResponse {
        download_url: Some(download_url),
        checksum_sha256: Some(object.checksum_sha256),
        size_bytes: Some(object.size_bytes),
    }))
}

pub(crate) async fn cache_upload(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((attempt_id, identity_digest)): Path<(String, String)>,
) -> Result<Json<CacheUploadSessionResponse>, ApiError> {
    let claim = require_cache_attempt(&state, &headers, &attempt_id).await?;
    require_cache_identity(&claim, &identity_digest)?;
    let object = state
        .metadata
        .runs()
        .begin_cache_upload(&identity_digest, unix_now()?)
        .await?;
    let signer = state.cache_presigner.as_ref().ok_or_else(|| {
        ApiError::infrastructure_unavailable("remote run cache is not configured")
    })?;
    let upload_url = signer
        .presign("PUT", &object.object_key, CACHE_URL_TTL_SECONDS)
        .map_err(|error| ApiError::internal_message(error.to_string()))?;
    Ok(Json(CacheUploadSessionResponse {
        upload_url,
        generation: object.generation,
    }))
}

pub(crate) async fn cache_commit(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((attempt_id, identity_digest)): Path<(String, String)>,
    Json(input): Json<CommitCacheUploadRequest>,
) -> Result<axum::http::StatusCode, ApiError> {
    let claim = require_cache_attempt(&state, &headers, &attempt_id).await?;
    require_cache_identity(&claim, &identity_digest)?;
    state
        .metadata
        .runs()
        .commit_cache_upload(
            &identity_digest,
            input.generation,
            &input.checksum_sha256,
            input.size_bytes,
            unix_now()?,
        )
        .await?;
    Ok(axum::http::StatusCode::NO_CONTENT)
}

fn require_cache_identity(
    claim: &scope_postgres::db::DispatchClaim,
    digest: &str,
) -> Result<(), ApiError> {
    let definition = claim
        .workflow_revision
        .definition()
        .job(&claim.job.key)
        .ok_or_else(|| ApiError::internal_message("attempt job definition is missing"))?;
    let namespace = CacheNamespace::workflow(claim.run.workflow.path(), &claim.job.key);
    let valid = definition.caches().iter().any(|cache| {
        CacheIdentity::new(
            claim.run.workflow.repository_id(),
            namespace.clone(),
            cache.clone(),
            &claim.job.pinned_container_image,
            CachePlatform::LinuxAmd64,
        )
        .is_ok_and(|identity| identity.digest() == digest)
    });
    if !valid {
        return Err(ApiError::bad_request(
            "cache identity does not belong to this attempt",
        ));
    }
    Ok(())
}

async fn require_cache_attempt(
    state: &AppState,
    headers: &HeaderMap,
    attempt_id: &str,
) -> Result<scope_postgres::db::DispatchClaim, ApiError> {
    state
        .metadata
        .runs()
        .authenticate_attempt_cache(attempt_id, &attempt_token_hash(headers)?, unix_now()?)
        .await
        .map_err(ApiError::from)
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
