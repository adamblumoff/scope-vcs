use crate::{
    auth::shoo::bearer_token,
    config::SCOPE_OPERATOR_TOKEN_ENV,
    db::MetadataResetEvent,
    domain::store::{RepoStorageCleanup, SourceBlob},
    error::ApiError,
    state::{AppState, CleanupDrainReport, drain_pending_cleanup},
};
use axum::{
    Json,
    extract::State,
    http::{HeaderMap, StatusCode},
};
use serde::{Deserialize, Serialize};

const METADATA_RESET_CONFIRMATION: &str = "reset-pre-alpha-metadata";

#[derive(Debug, Serialize)]
pub(crate) struct AdminCleanupStatusResponse {
    pending_cleanup: PendingCleanupResponse,
    failed_object_deletes: SourceBlobCleanupQueueResponse,
    metadata_resets: MetadataResetEventsResponse,
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
struct MetadataResetEventsResponse {
    count: usize,
    events: Vec<MetadataResetEvent>,
}

#[derive(Debug, Serialize)]
pub(crate) struct CleanupDrainResponse {
    status: &'static str,
    report: CleanupDrainReport,
}

#[derive(Debug, Deserialize)]
pub(crate) struct MetadataResetRequest {
    confirm: String,
    reason: Option<String>,
}

#[derive(Debug, Serialize)]
pub(crate) struct MetadataResetResponse {
    event: MetadataResetEvent,
}

pub(crate) async fn get_cleanup_status(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<AdminCleanupStatusResponse>, ApiError> {
    ensure_operator(&state, &headers)?;
    cleanup_status(&state).map(Json)
}

pub(crate) async fn drain_cleanup(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<(StatusCode, Json<CleanupDrainResponse>), ApiError> {
    ensure_operator(&state, &headers)?;
    let report = drain_pending_cleanup(&state)?;
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

pub(crate) async fn reset_metadata(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(request): Json<MetadataResetRequest>,
) -> Result<Json<MetadataResetResponse>, ApiError> {
    ensure_operator(&state, &headers)?;
    if request.confirm != METADATA_RESET_CONFIRMATION {
        return Err(ApiError::bad_request(format!(
            "confirm must be {METADATA_RESET_CONFIRMATION}"
        )));
    }

    let reason = request
        .reason
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or("operator requested pre-alpha metadata reset");
    let event = state.metadata.reset_catalog(reason)?;
    Ok(Json(MetadataResetResponse { event }))
}

fn cleanup_status(state: &AppState) -> Result<AdminCleanupStatusResponse, ApiError> {
    let (repo_storage, source_blob_deletes) = state.metadata.read(|catalog| {
        Ok((
            catalog.pending_repo_storage_deletions.clone(),
            catalog.pending_source_blob_deletions.clone(),
        ))
    })?;
    let source_blob_deletes = SourceBlobCleanupQueueResponse::from_blobs(&source_blob_deletes);
    let reset_events = state.metadata.metadata_reset_events()?;
    Ok(AdminCleanupStatusResponse {
        pending_cleanup: PendingCleanupResponse {
            repo_storage: RepoStorageCleanupQueueResponse::from_cleanups(&repo_storage),
            source_blob_deletes: source_blob_deletes.clone(),
        },
        failed_object_deletes: source_blob_deletes,
        metadata_resets: MetadataResetEventsResponse {
            count: reset_events.len(),
            events: reset_events,
        },
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
                    object_key: blob.object_key.clone(),
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
