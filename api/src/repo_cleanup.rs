use crate::{error::ApiError, persistence::unix_now, state::AppState};
use scope_domain::store::{SourceBlob, repo_id};
use serde::Serialize;
use std::collections::BTreeSet;

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub(crate) struct CleanupDrainReport {
    pub(crate) repo_storage: RepoStorageCleanupDrainReport,
    pub(crate) source_blobs: SourceBlobCleanupDrainReport,
}

impl CleanupDrainReport {
    pub(crate) fn has_failures(&self) -> bool {
        self.repo_storage.has_failures() || self.source_blobs.has_failures()
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
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

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub(crate) struct RepoStorageCleanupFailure {
    pub(crate) owner_handle: String,
    pub(crate) repo_name: String,
    pub(crate) error: String,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
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

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
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

#[derive(Debug, Default)]
struct SourceBlobStorageDeleteReport {
    deleted_keys: BTreeSet<String>,
    failed: Vec<SourceBlobCleanupFailure>,
}

fn unreferenced_source_blobs_by_key(
    referenced: &BTreeSet<scope_domain::content_ref::ContentRef>,
    blobs: &[SourceBlob],
) -> Vec<SourceBlob> {
    let mut unreferenced = std::collections::BTreeMap::new();
    for blob in blobs {
        let object_key = scope_object_store::object_key(blob);
        if !referenced.contains(&blob.content_ref) {
            unreferenced.entry(object_key).or_insert(blob);
        }
    }
    unreferenced.values().cloned().cloned().collect()
}

fn delete_source_blob_storage(
    state: &AppState,
    blobs: &[SourceBlob],
) -> SourceBlobStorageDeleteReport {
    let mut report = SourceBlobStorageDeleteReport::default();
    for blob in blobs {
        match delete_source_blob_storage_entry(state, blob) {
            Ok(()) => {
                report
                    .deleted_keys
                    .insert(scope_object_store::object_key(blob));
            }
            Err(error) => {
                tracing::warn!(
                    ?error,
                    object_key = %scope_object_store::object_key(blob),
                    "failed to clean source blob storage"
                );
                report
                    .failed
                    .push(SourceBlobCleanupFailure::from_blob(blob, error));
            }
        }
    }
    report
}

fn delete_source_blob_storage_entry(state: &AppState, blob: &SourceBlob) -> Result<(), ApiError> {
    Ok(state
        .object_store
        .delete(&scope_object_store::object_key(blob))?)
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
                    crate::git::storage::delete_repo_storage(
                        &state,
                        &cleanup.owner_handle,
                        &cleanup.repo_name,
                    ),
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
    let metadata = state.metadata.clone();
    let cleanup_store = metadata.cleanup();
    let now_unix = unix_now()?;
    let state = state.clone();
    let batch = cleanup_store
        .source_blob_cleanup_batch(now_unix, &crate::persistence_ids::generate_persistence_id)
        .await?;
    let unreferenced =
        unreferenced_source_blobs_by_key(&batch.referenced_content_refs, &batch.pending);
    let mut report = SourceBlobCleanupDrainReport {
        skipped_referenced: batch.pending.len().saturating_sub(unreferenced.len()),
        attempted: unreferenced.len(),
        ..Default::default()
    };
    let delete_report = delete_source_blob_storage(&state, &unreferenced);
    report.deleted = delete_report.deleted_keys.len();
    report.failed_object_deletes = delete_report.failed;
    let retained = unreferenced
        .into_iter()
        .filter(|blob| {
            !delete_report
                .deleted_keys
                .contains(&scope_object_store::object_key(blob))
        })
        .collect::<Vec<_>>();
    report.retained = retained.len();
    cleanup_store
        .finish_source_blob_cleanup(
            batch,
            &retained,
            now_unix,
            &crate::persistence_ids::generate_persistence_id,
        )
        .await?;
    Ok(report)
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
