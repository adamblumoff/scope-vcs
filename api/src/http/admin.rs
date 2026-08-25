use crate::{
    auth::clerk::bearer_token,
    config::SCOPE_OPERATOR_TOKEN_ENV,
    error::ApiError,
    state::AppState,
    use_cases::content_cleanup::{
        self, CleanupDrainReport, RepoStorageCleanupDrainReport, RepoStorageCleanupFailure,
        SourceBlobCleanupDrainReport, SourceBlobCleanupFailure,
    },
};
use axum::{
    Json,
    extract::State,
    http::{HeaderMap, StatusCode},
};
use scope_domain::content::SourceBlob;
use scope_domain::repo_actions::RepoStorageCleanup;
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
    report: CleanupDrainReportResponse,
}

#[derive(Debug, Serialize)]
struct CleanupDrainReportResponse {
    repo_storage: RepoStorageCleanupDrainReportResponse,
    source_blobs: SourceBlobCleanupDrainReportResponse,
}

#[derive(Debug, Serialize)]
struct RepoStorageCleanupDrainReportResponse {
    attempted: usize,
    deleted: usize,
    retained: usize,
    failed: Vec<RepoStorageCleanupFailureResponse>,
}

#[derive(Debug, Serialize)]
struct RepoStorageCleanupFailureResponse {
    owner_handle: String,
    repo_name: String,
    error: String,
}

#[derive(Debug, Serialize)]
struct SourceBlobCleanupDrainReportResponse {
    attempted: usize,
    deleted: usize,
    retained: usize,
    skipped_referenced: usize,
    failed_object_deletes: Vec<SourceBlobCleanupFailureResponse>,
}

#[derive(Debug, Serialize)]
struct SourceBlobCleanupFailureResponse {
    object_key: String,
    sha256: String,
    git_oid: String,
    size_bytes: u64,
    error: String,
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
    let report = content_cleanup::drain_pending_cleanup(&state).await?;
    let has_failures = report.has_failures();
    Ok((
        if has_failures {
            StatusCode::SERVICE_UNAVAILABLE
        } else {
            StatusCode::OK
        },
        Json(CleanupDrainResponse {
            status: if has_failures { "failed" } else { "drained" },
            report: CleanupDrainReportResponse::from_report(report),
        }),
    ))
}

async fn cleanup_status(state: &AppState) -> Result<AdminCleanupStatusResponse, ApiError> {
    let status = content_cleanup::cleanup_status(state).await?;
    let source_blob_deletes =
        SourceBlobCleanupQueueResponse::from_blobs(&status.source_blob_deletes);
    Ok(AdminCleanupStatusResponse {
        pending_cleanup: PendingCleanupResponse {
            repo_storage: RepoStorageCleanupQueueResponse::from_cleanups(&status.repo_storage),
            source_blob_deletes: source_blob_deletes.clone(),
        },
        failed_object_deletes: source_blob_deletes,
    })
}

impl CleanupDrainReportResponse {
    fn from_report(report: CleanupDrainReport) -> Self {
        Self {
            repo_storage: RepoStorageCleanupDrainReportResponse::from_report(report.repo_storage),
            source_blobs: SourceBlobCleanupDrainReportResponse::from_report(report.source_blobs),
        }
    }
}

impl RepoStorageCleanupDrainReportResponse {
    fn from_report(report: RepoStorageCleanupDrainReport) -> Self {
        Self {
            attempted: report.attempted,
            deleted: report.deleted,
            retained: report.retained,
            failed: report
                .failed
                .into_iter()
                .map(RepoStorageCleanupFailureResponse::from_failure)
                .collect(),
        }
    }
}

impl RepoStorageCleanupFailureResponse {
    fn from_failure(failure: RepoStorageCleanupFailure) -> Self {
        Self {
            owner_handle: failure.owner_handle,
            repo_name: failure.repo_name,
            error: failure.error,
        }
    }
}

impl SourceBlobCleanupDrainReportResponse {
    fn from_report(report: SourceBlobCleanupDrainReport) -> Self {
        Self {
            attempted: report.attempted,
            deleted: report.deleted,
            retained: report.retained,
            skipped_referenced: report.skipped_referenced,
            failed_object_deletes: report
                .failed_object_deletes
                .into_iter()
                .map(SourceBlobCleanupFailureResponse::from_failure)
                .collect(),
        }
    }
}

impl SourceBlobCleanupFailureResponse {
    fn from_failure(failure: SourceBlobCleanupFailure) -> Self {
        Self {
            object_key: failure.object_key,
            sha256: failure.sha256,
            git_oid: failure.git_oid,
            size_bytes: failure.size_bytes,
            error: failure.error,
        }
    }
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
        ApiError::infrastructure_unavailable(format!(
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
