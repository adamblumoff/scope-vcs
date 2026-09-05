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
    AppendAttemptLogRequest, AttemptCacheKeyMaterial, AttemptConclusionRequest,
    AttemptHeartbeatRequest, AttemptHeartbeatResponse, AttemptRecoveryStatusResponse, AttemptState,
    AttemptStatusResponse, AttemptStepStatusResponse, CacheColdReason, CacheFinalState,
    CachePreparation, ClaimRuntimeResponse, CompleteAttemptRequest, CompleteAttemptStepRequest,
    ReportAttemptCacheFinalizationsRequest, ReportAttemptCachePreparationsRequest, RunChangeKind,
    RunJobResponse, StepConclusionRequest, StepState, WorkflowCache, WorkflowCacheKeyInputs,
    WorkflowContainer, WorkflowJob, WorkflowStep,
};
use scope_domain::{
    runs::cache::identity::{CacheIdentity, CacheNamespace, CachePlatform},
    runs::log::RunLogChunk,
    runs::step::{AttemptConclusion, RunAttemptStep, StepConclusion},
};

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
    let cache_grant = issue_cache_grant(&state, &claim, &[])?;
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
    Json(input): Json<AttemptHeartbeatRequest>,
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
    let cache_grant = issue_cache_grant(&state, &claim, &input.cache_keys)?;
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
            input.authorization_ms,
            input.wall_ms,
            input
                .caches
                .into_iter()
                .map(|cache| scope_postgres::db::AttemptCachePreparationCommand {
                    cache_name: cache.cache_name,
                    identity_digest: cache.identity_digest,
                    preparation: domain_cache_preparation(cache.preparation),
                    key_ms: cache.key_ms,
                    metadata_ms: cache.metadata_ms,
                    size_bytes: cache.size_bytes,
                    download_verify_ms: cache.download_verify_ms,
                    sync_ms: cache.sync_ms,
                    extraction_ms: cache.extraction_ms,
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
                        final_state: domain_cache_final_state(cache.final_state),
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
    let materialized =
        materialize_run_source_bundle(&state, &source, MAX_SOURCE_BUNDLE_BYTES).await?;
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
        state: attempt_state(claim.attempt.state),
        cancellation_requested: claim.run.cancellation_requested,
        lease_expires_at_unix: claim.attempt.lease_expires_at_unix,
    }
}

fn claimed_job_definition(claim: &scope_postgres::db::DispatchClaim) -> WorkflowJob {
    let job = claim
        .workflow_revision
        .definition()
        .job(&claim.job.key)
        .expect("claimed run job definition must exist");
    WorkflowJob {
        id: job.id().as_str().to_string(),
        needs: job
            .needs()
            .iter()
            .map(|dependency| dependency.as_str().to_string())
            .collect(),
        container: WorkflowContainer {
            image: job.container().image().to_string(),
        },
        timeout_seconds: job.timeout_seconds(),
        caches: job
            .caches()
            .iter()
            .map(|cache| WorkflowCache {
                name: cache.as_str().to_string(),
                path: cache.mount_path().to_string(),
                format: cache.format().to_string(),
                compatibility: workflow_cache_inputs(cache.compatibility_inputs()),
                exact: workflow_cache_inputs(cache.exact_inputs()),
            })
            .collect(),
        environment: job.environment().clone(),
        steps: job
            .steps()
            .iter()
            .map(|step| WorkflowStep {
                name: step.name().to_string(),
                run: step.run().to_string(),
            })
            .collect(),
    }
}

fn issue_cache_grant(
    state: &AppState,
    claim: &scope_postgres::db::DispatchClaim,
    materials: &[AttemptCacheKeyMaterial],
) -> Result<String, ApiError> {
    let job_definition = claim
        .workflow_revision
        .definition()
        .job(&claim.job.key)
        .expect("claimed run job definition must exist");
    let namespace = CacheNamespace::workflow(claim.run.workflow.path(), &claim.job.key);
    let mut names = std::collections::BTreeSet::new();
    let allowed_caches = materials
        .iter()
        .map(|material| {
            if !names.insert(material.cache_name.as_str()) {
                return Err(ApiError::bad_request(
                    "cache key material contains a duplicate name",
                ));
            }
            let cache = job_definition
                .caches()
                .iter()
                .find(|cache| cache.as_str() == material.cache_name)
                .ok_or_else(|| {
                    ApiError::bad_request("cache key material does not belong to the claimed job")
                })?;
            let identity = CacheIdentity::new(
                claim.run.workflow.repository_id(),
                namespace.clone(),
                cache.clone(),
                CachePlatform::LinuxAmd64,
                &material.compatibility_inputs_digest,
                &material.exact_inputs_digest,
            )?;
            Ok(scope_cache_contract::AuthorizedCache {
                exact_identity_digest: scope_cache_domain::CacheDigest::parse(
                    identity.exact_digest(),
                )
                .map_err(ApiError::bad_request)?,
                compatibility_group_digest: scope_cache_domain::CacheDigest::parse(
                    identity.compatibility_group_digest(),
                )
                .map_err(ApiError::bad_request)?,
            })
        })
        .collect::<Result<Vec<_>, ApiError>>()?;
    state
        .cache_grants
        .issue(
            claim.attempt.id.clone(),
            scope_cache_domain::RepositoryId::parse(claim.run.workflow.repository_id().to_string())
                .map_err(ApiError::bad_request)?,
            allowed_caches,
            claim.attempt.lease_expires_at_unix,
        )
        .map_err(|error| ApiError::internal_message(error.to_string()))
}

fn attempt_step_status(step: &RunAttemptStep) -> AttemptStepStatusResponse {
    AttemptStepStatusResponse {
        step_index: step.step_index,
        state: step_state(step.state),
        started_at_unix: step.started_at_unix,
        completed_at_unix: step.completed_at_unix,
        exit_code: step.exit_code,
    }
}

fn workflow_cache_inputs(
    inputs: &scope_domain::runs::cache::definition::CacheKeyInputs,
) -> WorkflowCacheKeyInputs {
    WorkflowCacheKeyInputs {
        files: inputs.files().to_vec(),
        environment: inputs.environment().to_vec(),
        source: inputs.includes_source(),
    }
}

fn attempt_state(state: scope_domain::runs::attempt::AttemptState) -> AttemptState {
    match state {
        scope_domain::runs::attempt::AttemptState::Dispatching => AttemptState::Dispatching,
        scope_domain::runs::attempt::AttemptState::Running => AttemptState::Running,
        scope_domain::runs::attempt::AttemptState::Succeeded => AttemptState::Succeeded,
        scope_domain::runs::attempt::AttemptState::Failed => AttemptState::Failed,
        scope_domain::runs::attempt::AttemptState::Canceled => AttemptState::Canceled,
        scope_domain::runs::attempt::AttemptState::Lost => AttemptState::Lost,
    }
}

fn step_state(state: scope_domain::runs::step::StepState) -> StepState {
    match state {
        scope_domain::runs::step::StepState::Pending => StepState::Pending,
        scope_domain::runs::step::StepState::Running => StepState::Running,
        scope_domain::runs::step::StepState::Succeeded => StepState::Succeeded,
        scope_domain::runs::step::StepState::Failed => StepState::Failed,
        scope_domain::runs::step::StepState::Canceled => StepState::Canceled,
        scope_domain::runs::step::StepState::Lost => StepState::Lost,
        scope_domain::runs::step::StepState::Skipped => StepState::Skipped,
    }
}

fn domain_cache_preparation(
    preparation: CachePreparation,
) -> scope_domain::runs::cache::observation::CachePreparation {
    match preparation {
        CachePreparation::Exact => scope_domain::runs::cache::observation::CachePreparation::Exact,
        CachePreparation::Compatible => scope_domain::runs::cache::observation::CachePreparation::Compatible,
        CachePreparation::Cold { reason } => scope_domain::runs::cache::observation::CachePreparation::Cold {
            reason: match reason {
                CacheColdReason::MetadataMissing => {
                    scope_domain::runs::cache::observation::CacheColdReason::MetadataMissing
                }
                CacheColdReason::MetadataInvalid => {
                    scope_domain::runs::cache::observation::CacheColdReason::MetadataInvalid
                }
                CacheColdReason::MetadataNotReady => {
                    scope_domain::runs::cache::observation::CacheColdReason::MetadataNotReady
                }
                CacheColdReason::VolumeMissing => {
                    scope_domain::runs::cache::observation::CacheColdReason::VolumeMissing
                }
                CacheColdReason::VolumeInvalid => {
                    scope_domain::runs::cache::observation::CacheColdReason::VolumeInvalid
                }
                CacheColdReason::BackingDirectoryMissing => {
                    scope_domain::runs::cache::observation::CacheColdReason::BackingDirectoryMissing
                }
            },
        },
    }
}

fn domain_cache_final_state(
    state: CacheFinalState,
) -> scope_domain::runs::cache::observation::CacheFinalState {
    match state {
        CacheFinalState::Pending => {
            scope_domain::runs::cache::observation::CacheFinalState::Pending
        }
        CacheFinalState::Ready => scope_domain::runs::cache::observation::CacheFinalState::Ready,
        CacheFinalState::Evicted => {
            scope_domain::runs::cache::observation::CacheFinalState::Evicted
        }
    }
}
