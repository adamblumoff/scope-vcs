use crate::{error::ApiError, persistence::unix_now, state::AppState};
use scope_domain::{content::SourceBlob, repo_actions::RepoStorageCleanup, repository::repo_id};

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct CleanupDrainReport {
    pub(crate) repo_storage: RepoStorageCleanupDrainReport,
    pub(crate) source_blobs: SourceBlobCleanupDrainReport,
}

impl CleanupDrainReport {
    pub(crate) fn has_failures(&self) -> bool {
        self.repo_storage.has_failures() || self.source_blobs.has_failures()
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct RepoStorageCleanupDrainReport {
    pub(crate) attempted: usize,
    pub(crate) deleted: usize,
    pub(crate) retained: usize,
    pub(crate) failed: Vec<RepoStorageCleanupFailure>,
}

impl RepoStorageCleanupDrainReport {
    fn has_failures(&self) -> bool {
        !self.failed.is_empty()
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct RepoStorageCleanupFailure {
    pub(crate) owner_handle: String,
    pub(crate) repo_name: String,
    pub(crate) error: String,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct SourceBlobCleanupDrainReport {
    pub(crate) attempted: usize,
    pub(crate) deleted: usize,
    pub(crate) retained: usize,
    pub(crate) skipped_referenced: usize,
    pub(crate) failed_object_deletes: Vec<SourceBlobCleanupFailure>,
}

impl SourceBlobCleanupDrainReport {
    fn has_failures(&self) -> bool {
        !self.failed_object_deletes.is_empty()
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct SourceBlobCleanupFailure {
    pub(crate) object_key: String,
    pub(crate) sha256: String,
    pub(crate) git_oid: String,
    pub(crate) size_bytes: u64,
    pub(crate) error: String,
}

impl SourceBlobCleanupFailure {
    fn from_blob(blob: &SourceBlob, error: ApiError) -> Self {
        Self {
            object_key: scope_object_store::object_key(blob),
            sha256: blob.sha256.clone(),
            git_oid: blob.git_oid.clone(),
            size_bytes: blob.size_bytes,
            error: error.into_operator_diagnostic(),
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct CleanupStatus {
    pub(crate) repo_storage: Vec<RepoStorageCleanup>,
    pub(crate) source_blob_deletes: Vec<SourceBlob>,
}

pub(crate) async fn cleanup_status(state: &AppState) -> Result<CleanupStatus, ApiError> {
    let (repo_storage, source_blob_deletes) =
        state.metadata.cleanup().pending_cleanup_queues().await?;
    Ok(CleanupStatus {
        repo_storage,
        source_blob_deletes,
    })
}

pub(crate) async fn drain_pending_cleanup(
    state: &AppState,
) -> Result<CleanupDrainReport, ApiError> {
    Ok(CleanupDrainReport {
        repo_storage: drain_pending_repo_storage_deletions_report(state).await?,
        source_blobs: drain_pending_source_blob_deletions_report(state).await?,
    })
}

pub(crate) async fn drain_pending_repo_storage_deletions_report(
    state: &AppState,
) -> Result<RepoStorageCleanupDrainReport, ApiError> {
    let metadata = state.metadata.clone();
    let cleanup_store = metadata.cleanup();
    let repository_store = metadata.repositories();
    let now_unix = unix_now()?;
    let state = state.clone();
    let batch = cleanup_store
        .repo_storage_cleanup_batch(now_unix, &crate::persistence_ids::generate_persistence_id)
        .await?;
    let mut report = RepoStorageCleanupDrainReport::default();
    let mut retained = Vec::new();
    for cleanup in &batch.pending {
        let cleanup_repo_id = repo_id(&cleanup.owner_handle, &cleanup.repo_name);
        let repository_store = repository_store.clone();
        let state = state.clone();
        let (live_repo, delete_result) = repository_store
            .with_repo_storage_lock(&cleanup_repo_id, || async {
                if repository_store.repository_exists(&cleanup_repo_id).await? {
                    return Ok::<_, ApiError>((true, Ok(())));
                }
                Ok::<_, ApiError>((
                    false,
                    crate::git::storage::delete_repo_storage(&state, cleanup),
                ))
            })
            .await?;
        if live_repo {
            retained.push(cleanup.clone());
            continue;
        }
        report.attempted += 1;
        match delete_result {
            Ok(()) => report.deleted += 1,
            Err(error) => {
                tracing::warn!(?error, owner = %cleanup.owner_handle, repo = %cleanup.repo_name, "failed to clean deleted repo filesystem storage");
                report.failed.push(RepoStorageCleanupFailure {
                    owner_handle: cleanup.owner_handle.clone(),
                    repo_name: cleanup.repo_name.clone(),
                    error: error.into_operator_diagnostic(),
                });
                retained.push(cleanup.clone());
            }
        }
    }
    report.retained = retained.len();
    cleanup_store
        .finish_repo_storage_cleanup(
            batch,
            &retained,
            now_unix,
            &crate::persistence_ids::generate_persistence_id,
        )
        .await?;
    Ok(report)
}

pub(crate) async fn drain_pending_repo_storage_deletions(state: &AppState) -> Result<(), ApiError> {
    let report = drain_pending_repo_storage_deletions_report(state).await?;
    match report.failed.first() {
        Some(failure) => Err(ApiError::infrastructure_unavailable(format!(
            "failed to clean deleted repo storage {}/{}: {}",
            failure.owner_handle, failure.repo_name, failure.error
        ))),
        None => Ok(()),
    }
}

pub(crate) async fn best_effort_drain_pending_repo_storage_deletions(state: &AppState) {
    if let Err(error) = drain_pending_repo_storage_deletions(state).await {
        tracing::warn!(?error, "failed to drain pending repo storage deletions");
    }
}

pub(crate) async fn persist_pending_source_blob_deletions(
    state: &AppState,
    blobs: &[SourceBlob],
) -> Result<(), ApiError> {
    if blobs.is_empty() {
        return Ok(());
    }
    let blobs = blobs.to_vec();
    Ok(state
        .metadata
        .cleanup()
        .queue_pending_source_blob_deletions(
            blobs,
            unix_now()?,
            &crate::persistence_ids::generate_persistence_id,
        )
        .await?)
}

pub(crate) async fn best_effort_cleanup_rollback_source_blobs(
    state: &AppState,
    blobs: &[SourceBlob],
) {
    if blobs.is_empty() {
        return;
    }
    if let Err(queue_error) = persist_pending_source_blob_deletions(state, blobs).await {
        tracing::warn!(?queue_error, "failed to queue rollback source blob cleanup");
    }
}

pub(crate) async fn drain_pending_source_blob_deletions_report(
    state: &AppState,
) -> Result<SourceBlobCleanupDrainReport, ApiError> {
    let now_unix = unix_now()?;
    let shared = scope_content_lifecycle::drain_source_blob_cleanup(
        &state.metadata,
        state.object_store.as_ref(),
        now_unix,
        &crate::persistence_ids::generate_persistence_id,
    )
    .await?;
    Ok(SourceBlobCleanupDrainReport {
        attempted: shared.attempted,
        deleted: shared.deleted,
        retained: shared.retained,
        skipped_referenced: shared.skipped_referenced,
        failed_object_deletes: shared
            .failed_object_deletes
            .into_iter()
            .map(|failure| {
                SourceBlobCleanupFailure::from_blob(&failure.blob, ApiError::from(failure.error))
            })
            .collect(),
    })
}

#[cfg(test)]
pub(crate) async fn drain_pending_orphan_objects(state: &AppState) -> Result<(), ApiError> {
    let report = drain_pending_source_blob_deletions_report(state).await?;
    match report.failed_object_deletes.first().map(|failure| {
        ApiError::infrastructure_unavailable(format!(
            "failed to clean source blob storage {}: {}",
            failure.object_key, failure.error
        ))
    }) {
        Some(error) => Err(error),
        None => Ok(()),
    }
}
