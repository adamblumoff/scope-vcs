use crate::{
    auth::clerk::bearer_token,
    config::SCOPE_OPERATOR_TOKEN_ENV,
    error::ApiError,
    repo_cleanup::{CleanupDrainReport, drain_pending_cleanup},
    state::AppState,
};
use axum::{
    Json,
    extract::State,
    http::{HeaderMap, StatusCode},
};
use scope_api_contract::{
    AdvanceRunnerProtocolCutoverRequest, CreateRunnerProtocolCanaryRequest,
    RunnerProtocolCanaryResponse, RunnerProtocolCutoverResponse,
};
use scope_domain::store::{RepoStorageCleanup, SourceBlob};
use serde::Serialize;

#[derive(Debug, Serialize)]
pub(crate) struct AdminCleanupStatusResponse {
    pending_cleanup: PendingCleanupResponse,
    failed_object_deletes: SourceBlobCleanupQueueResponse,
}

#[derive(Debug, Serialize)]
struct PendingCleanupResponse {
    repo_storage: RepoStorageCleanupQueueResponse,
    source_blob_deletes: SourceBlobCleanupQueueResponse,
}

#[derive(Debug, Serialize)]
struct RepoStorageCleanupQueueResponse {
    count: usize,
    repos: Vec<RepoStorageCleanupResponse>,
}

#[derive(Debug, Serialize)]
struct RepoStorageCleanupResponse {
    owner_handle: String,
    repo_name: String,
}

#[derive(Clone, Debug, Serialize)]
struct SourceBlobCleanupQueueResponse {
    count: usize,
    objects: Vec<SourceBlobCleanupResponse>,
}

#[derive(Clone, Debug, Serialize)]
struct SourceBlobCleanupResponse {
    object_key: String,
    sha256: String,
    git_oid: String,
    size_bytes: u64,
}

#[derive(Debug, Serialize)]
pub(crate) struct CleanupDrainResponse {
    status: &'static str,
    report: CleanupDrainReport,
}

pub(crate) async fn get_cleanup_status(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<AdminCleanupStatusResponse>, ApiError> {
    ensure_operator(&state, &headers)?;
    cleanup_status(&state).await.map(Json)
}

pub(crate) async fn drain_cleanup(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<(StatusCode, Json<CleanupDrainResponse>), ApiError> {
    ensure_operator(&state, &headers)?;
    let report = drain_pending_cleanup(&state).await?;
    let has_failures = report.has_failures();
    Ok((
        if has_failures {
            StatusCode::SERVICE_UNAVAILABLE
        } else {
            StatusCode::OK
        },
        Json(CleanupDrainResponse {
            status: if has_failures { "failed" } else { "drained" },
            report,
        }),
    ))
}

pub(crate) async fn get_runner_protocol_cutover(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<RunnerProtocolCutoverResponse>, ApiError> {
    ensure_operator(&state, &headers)?;
    Ok(Json(runner_protocol_cutover_response(
        state.metadata.admin().runner_protocol_cutover().await?,
    )))
}

pub(crate) async fn advance_runner_protocol_cutover(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(input): Json<AdvanceRunnerProtocolCutoverRequest>,
) -> Result<Json<RunnerProtocolCutoverResponse>, ApiError> {
    ensure_operator(&state, &headers)?;
    Ok(Json(runner_protocol_cutover_response(
        state
            .metadata
            .admin()
            .advance_runner_protocol_cutover(input.state, crate::persistence::unix_now()?)
            .await?,
    )))
}

pub(crate) async fn create_runner_protocol_canary(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(input): Json<CreateRunnerProtocolCanaryRequest>,
) -> Result<Json<RunnerProtocolCutoverResponse>, ApiError> {
    ensure_operator(&state, &headers)?;
    Ok(Json(runner_protocol_cutover_response(
        state
            .metadata
            .admin()
            .create_runner_protocol_canary(
                input.phase,
                &input.runner_id,
                &input.run_id,
                crate::persistence::unix_now()?,
            )
            .await?,
    )))
}

fn runner_protocol_cutover_response(
    snapshot: scope_postgres::db::RunnerProtocolCutoverSnapshot,
) -> RunnerProtocolCutoverResponse {
    RunnerProtocolCutoverResponse {
        state: snapshot.cutover.state(),
        generation: snapshot.canary_generation,
        canaries: snapshot
            .canaries
            .into_iter()
            .map(|canary| RunnerProtocolCanaryResponse {
                generation: canary.generation().get(),
                phase: canary.phase(),
                runner_id: canary.runner_id().to_string(),
                run_id: canary.run_id().to_string(),
                status: canary.status(),
            })
            .collect(),
    }
}

async fn cleanup_status(state: &AppState) -> Result<AdminCleanupStatusResponse, ApiError> {
    let (repo_storage, source_blob_deletes) =
        state.metadata.cleanup().pending_cleanup_queues().await?;
    let source_blob_deletes = SourceBlobCleanupQueueResponse::from_blobs(&source_blob_deletes);
    Ok(AdminCleanupStatusResponse {
        pending_cleanup: PendingCleanupResponse {
            repo_storage: RepoStorageCleanupQueueResponse::from_cleanups(&repo_storage),
            source_blob_deletes: source_blob_deletes.clone(),
        },
        failed_object_deletes: source_blob_deletes,
    })
}

impl RepoStorageCleanupQueueResponse {
    fn from_cleanups(cleanups: &[RepoStorageCleanup]) -> Self {
        Self {
            count: cleanups.len(),
            repos: cleanups
                .iter()
                .map(|cleanup| RepoStorageCleanupResponse {
                    owner_handle: cleanup.owner_handle.clone(),
                    repo_name: cleanup.repo_name.clone(),
                })
                .collect(),
        }
    }
}

impl SourceBlobCleanupQueueResponse {
    fn from_blobs(blobs: &[SourceBlob]) -> Self {
        Self {
            count: blobs.len(),
            objects: blobs
                .iter()
                .map(|blob| SourceBlobCleanupResponse {
                    object_key: scope_object_store::object_key(blob),
                    sha256: blob.sha256.clone(),
                    git_oid: blob.git_oid.clone(),
                    size_bytes: blob.size_bytes,
                })
                .collect(),
        }
    }
}

fn ensure_operator(state: &AppState, headers: &HeaderMap) -> Result<(), ApiError> {
    let expected = state.operator_token.as_deref().ok_or_else(|| {
        ApiError::service_unavailable(format!(
            "{SCOPE_OPERATOR_TOKEN_ENV} is required for admin operations"
        ))
    })?;
    let Some(actual) = bearer_token(headers)? else {
        return Err(ApiError::unauthorized("operator token required"));
    };
    if !constant_time_eq(expected.as_bytes(), actual.as_bytes()) {
        return Err(ApiError::unauthorized("invalid operator token"));
    }
    Ok(())
}

fn constant_time_eq(left: &[u8], right: &[u8]) -> bool {
    let mut diff = left.len() ^ right.len();
    for index in 0..left.len().max(right.len()) {
        let left_byte = left.get(index).copied().unwrap_or(0);
        let right_byte = right.get(index).copied().unwrap_or(0);
        diff |= usize::from(left_byte ^ right_byte);
    }
    diff == 0
}
