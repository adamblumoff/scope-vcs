use crate::domain::policy::PrincipalKind;
use crate::domain::policy::{Principal, ScopePath};
use crate::domain::projection_views::has_visible_projected_files;
use crate::domain::store::{
    AppCatalog, RepoPublicationState, RepoRole, SourceBlob, StoredRepository, repo_id,
};
use crate::{
    auth::clerk::ClerkVerifier,
    config::{SCOPE_OPERATOR_TOKEN_ENV, data_dir, git_repo_root, non_empty_env},
    db::MetadataStore,
    error::ApiError,
    object_store::{EncryptedObjectStore, ObjectStore, S3ObjectStore},
    persistence::ensure_private_dir,
};
use serde::Serialize;
use std::{
    collections::{BTreeMap, BTreeSet},
    path::PathBuf,
    sync::Arc,
};

#[derive(Clone)]
pub struct AppState {
    pub(crate) metadata: MetadataStore,
    pub(crate) data_dir: Arc<PathBuf>,
    pub(crate) clerk: ClerkVerifier,
    pub(crate) object_store: Arc<dyn ObjectStore>,
    pub(crate) operator_token: Option<Arc<str>>,
}

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
            object_key: blob.object_key.clone(),
            sha256: blob.sha256.clone(),
            git_oid: blob.git_oid.clone(),
            size_bytes: blob.size_bytes,
            error: error.message,
        }
    }
}

#[derive(Debug, Default)]
struct SourceBlobStorageDeleteReport {
    deleted_keys: BTreeSet<String>,
    failed: Vec<SourceBlobCleanupFailure>,
}

impl SourceBlobStorageDeleteReport {
    fn first_error(&self) -> Option<ApiError> {
        self.failed.first().map(|failure| {
            ApiError::service_unavailable(format!(
                "failed to clean source blob storage {}: {}",
                failure.object_key, failure.error
            ))
        })
    }
}

impl AppState {
    pub fn from_env() -> anyhow::Result<Self> {
        let repo_root = git_repo_root();
        let data_dir = data_dir(&repo_root);
        ensure_private_dir(&data_dir).map_err(|error| anyhow::anyhow!(error.message))?;

        let state = Self {
            metadata: MetadataStore::connect_from_env()?,
            data_dir: Arc::new(data_dir),
            clerk: ClerkVerifier::from_env(),
            object_store: Arc::new(EncryptedObjectStore::from_env(Arc::new(
                S3ObjectStore::from_env()?,
            ))?),
            operator_token: non_empty_env(SCOPE_OPERATOR_TOKEN_ENV).map(Arc::from),
        };
        best_effort_drain_pending_repo_storage_deletions(&state);
        best_effort_drain_pending_source_blob_deletions(&state);
        Ok(state)
    }

    #[cfg(test)]
    pub(crate) fn test_state() -> Self {
        use crate::{domain::store::app_catalog, persistence::test_data_dir};

        Self {
            metadata: MetadataStore::memory(app_catalog()),
            data_dir: Arc::new(test_data_dir()),
            clerk: ClerkVerifier::new(
                Some("https://clerk.test".to_string()),
                Some("http://127.0.0.1/.well-known/jwks.json".to_string()),
            ),
            object_store: Arc::new(crate::object_store::MemoryObjectStore::new()),
            operator_token: None,
        }
    }

    pub(crate) fn git_cache_root(&self) -> Result<PathBuf, ApiError> {
        ensure_private_dir(&self.data_dir)?;
        let cache_root = self.data_dir.join("git-cache");
        ensure_private_dir(&cache_root)?;
        Ok(cache_root)
    }
}

pub(crate) fn find_repo(
    state: &AppState,
    owner: &str,
    name: &str,
) -> Result<StoredRepository, ApiError> {
    state
        .metadata
        .repository(owner, name)?
        .ok_or_else(|| ApiError::not_found(format!("repo {owner}/{name} not found")))
}

pub(crate) fn ensure_repo_read(
    state: &AppState,
    repo: &StoredRepository,
    principal: &Principal,
) -> Result<(), ApiError> {
    if can_read_path(state, repo, principal, &ScopePath::root())?
        || (repo.record.publication_state == RepoPublicationState::Published
            && has_visible_projected_files(repo, principal))
    {
        Ok(())
    } else {
        Err(ApiError::not_found(format!(
            "repo {} not found",
            repo.record.id
        )))
    }
}

pub(crate) fn ensure_owner(
    state: &AppState,
    repo: &StoredRepository,
    principal: &Principal,
) -> Result<(), ApiError> {
    if role_for_principal(state, repo, principal)? == Some(RepoRole::Owner) {
        Ok(())
    } else {
        Err(ApiError::forbidden("owner role required"))
    }
}

pub(crate) fn role_for_principal(
    _state: &AppState,
    repo: &StoredRepository,
    principal: &Principal,
) -> Result<Option<RepoRole>, ApiError> {
    if principal.kind != PrincipalKind::Public && principal.id == repo.record.owner_user_id {
        return Ok(Some(RepoRole::Owner));
    }

    Ok(repo
        .memberships
        .iter()
        .find(|membership| membership.user_id == principal.id)
        .map(|membership| membership.role))
}

pub(crate) fn can_read_path(
    _state: &AppState,
    repo: &StoredRepository,
    principal: &Principal,
    path: &ScopePath,
) -> Result<bool, ApiError> {
    if principal.kind == crate::domain::policy::PrincipalKind::Public {
        return Ok(
            repo.record.publication_state == RepoPublicationState::Published
                && repo.policy.can_read(principal, path),
        );
    }

    let Some(role) = role_for_principal(_state, repo, principal)? else {
        return Ok(false);
    };

    let lifecycle_allows_read =
        repo.record.publication_state == RepoPublicationState::Published || role == RepoRole::Owner;

    Ok(lifecycle_allows_read && repo.policy.can_read(principal, path))
}

pub(crate) fn can_write_path(
    _state: &AppState,
    repo: &StoredRepository,
    principal: &Principal,
    path: &ScopePath,
) -> Result<bool, ApiError> {
    let can_write = role_for_principal(_state, repo, principal)?
        .is_some_and(|role| role >= RepoRole::Writer)
        && can_read_path(_state, repo, principal, path)?;
    Ok(can_write)
}

pub(crate) fn repo_source_blobs(repo: &StoredRepository) -> Vec<SourceBlob> {
    repo.source_blobs()
}

pub(crate) fn delete_unreferenced_source_blobs(
    state: &AppState,
    blobs: &[SourceBlob],
) -> Result<(), ApiError> {
    let blobs = blobs.to_vec();
    let blobs = state
        .metadata
        .read(move |catalog| Ok(unreferenced_source_blobs(catalog, &blobs)))?;
    let report = delete_source_blob_storage(state, &blobs);
    match report.first_error() {
        Some(error) => Err(error),
        None => Ok(()),
    }
}

fn unreferenced_source_blobs(catalog: &AppCatalog, blobs: &[SourceBlob]) -> Vec<SourceBlob> {
    let referenced = catalog
        .repositories
        .values()
        .flat_map(repo_source_blobs)
        .map(|blob| blob.object_key)
        .collect::<std::collections::BTreeSet<_>>();
    let mut unreferenced = std::collections::BTreeMap::new();
    for blob in blobs {
        if !referenced.contains(blob.object_key.as_str()) {
            unreferenced.entry(blob.object_key.as_str()).or_insert(blob);
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
                report.deleted_keys.insert(blob.object_key.clone());
            }
            Err(error) => {
                tracing::warn!(
                    ?error,
                    object_key = %blob.object_key,
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
    crate::git::storage::delete_raw_git_snapshot_cache(state, blob)?;
    state.object_store.delete(&blob.object_key)
}

pub(crate) fn queue_source_blob_deletions(
    catalog: &mut AppCatalog,
    blobs: impl IntoIterator<Item = SourceBlob>,
) {
    let mut queued = catalog
        .pending_source_blob_deletions
        .iter()
        .map(|blob| blob.object_key.clone())
        .collect::<std::collections::BTreeSet<_>>();
    for blob in blobs {
        if queued.insert(blob.object_key.clone()) {
            catalog.pending_source_blob_deletions.push(blob);
        }
    }
}

pub(crate) fn drain_pending_cleanup(state: &AppState) -> Result<CleanupDrainReport, ApiError> {
    Ok(CleanupDrainReport {
        repo_storage: drain_pending_repo_storage_deletions_report(state)?,
        source_blobs: drain_pending_source_blob_deletions_report(state)?,
    })
}

pub(crate) fn drain_pending_repo_storage_deletions_report(
    state: &AppState,
) -> Result<RepoStorageCleanupDrainReport, ApiError> {
    let metadata = state.metadata.clone();
    let state = state.clone();
    metadata.update(move |catalog| {
        let mut report = RepoStorageCleanupDrainReport::default();
        let mut retained = Vec::new();
        for cleanup in std::mem::take(&mut catalog.pending_repo_storage_deletions) {
            if catalog
                .repositories
                .contains_key(&repo_id(&cleanup.owner_handle, &cleanup.repo_name))
            {
                retained.push(cleanup);
                continue;
            }

            report.attempted += 1;
            match crate::git::storage::delete_repo_storage(
                &state,
                &cleanup.owner_handle,
                &cleanup.repo_name,
            ) {
                Ok(()) => {
                    report.deleted += 1;
                }
                Err(error) => {
                    tracing::warn!(
                        ?error,
                        owner = %cleanup.owner_handle,
                        repo = %cleanup.repo_name,
                        "failed to clean deleted repo filesystem storage"
                    );
                    report.failed.push(RepoStorageCleanupFailure {
                        owner_handle: cleanup.owner_handle.clone(),
                        repo_name: cleanup.repo_name.clone(),
                        error: error.message,
                    });
                    retained.push(cleanup);
                }
            }
        }
        report.retained = retained.len();
        catalog.pending_repo_storage_deletions = retained;
        Ok(report)
    })
}

pub(crate) fn drain_pending_repo_storage_deletions(state: &AppState) -> Result<(), ApiError> {
    let report = drain_pending_repo_storage_deletions_report(state)?;
    match report.failed.first() {
        Some(failure) => Err(ApiError::service_unavailable(format!(
            "failed to clean deleted repo storage {}/{}: {}",
            failure.owner_handle, failure.repo_name, failure.error
        ))),
        None => Ok(()),
    }
}

pub(crate) fn best_effort_drain_pending_repo_storage_deletions(state: &AppState) {
    if let Err(error) = drain_pending_repo_storage_deletions(state) {
        tracing::warn!(?error, "failed to drain pending repo storage deletions");
    }
}

pub(crate) fn persist_pending_source_blob_deletions(
    state: &AppState,
    blobs: &[SourceBlob],
) -> Result<(), ApiError> {
    if blobs.is_empty() {
        return Ok(());
    }
    let blobs = blobs.to_vec();
    state.metadata.update(move |catalog| {
        queue_source_blob_deletions(catalog, blobs);
        Ok(())
    })
}

pub(crate) fn best_effort_cleanup_rollback_source_blobs(state: &AppState, blobs: &[SourceBlob]) {
    if blobs.is_empty() {
        return;
    }
    match persist_pending_source_blob_deletions(state, blobs) {
        Ok(()) => best_effort_drain_pending_source_blob_deletions(state),
        Err(queue_error) => {
            tracing::warn!(?queue_error, "failed to queue rollback source blob cleanup");
            if let Err(delete_error) = delete_unreferenced_source_blobs(state, blobs) {
                tracing::warn!(
                    ?delete_error,
                    "failed to delete rollback source blobs without queued retry"
                );
            }
        }
    }
}

pub(crate) fn drain_pending_source_blob_deletions_report(
    state: &AppState,
) -> Result<SourceBlobCleanupDrainReport, ApiError> {
    let metadata = state.metadata.clone();
    let state = state.clone();
    metadata.update(move |catalog| {
        let mut report = SourceBlobCleanupDrainReport::default();
        let pending = std::mem::take(&mut catalog.pending_source_blob_deletions);
        if pending.is_empty() {
            return Ok(report);
        }

        let unreferenced = unreferenced_source_blobs(catalog, &pending);
        report.skipped_referenced = pending.len().saturating_sub(unreferenced.len());
        report.attempted = unreferenced.len();
        let delete_report = delete_source_blob_storage(&state, &unreferenced);
        report.deleted = delete_report.deleted_keys.len();
        report.failed_object_deletes = delete_report.failed;
        catalog.pending_source_blob_deletions = unreferenced
            .into_iter()
            .filter(|blob| !delete_report.deleted_keys.contains(&blob.object_key))
            .collect();
        report.retained = catalog.pending_source_blob_deletions.len();
        Ok(report)
    })
}

pub(crate) fn drain_pending_source_blob_deletions(state: &AppState) -> Result<(), ApiError> {
    let report = drain_pending_source_blob_deletions_report(state)?;
    match report.failed_object_deletes.first().map(|failure| {
        ApiError::service_unavailable(format!(
            "failed to clean source blob storage {}: {}",
            failure.object_key, failure.error
        ))
    }) {
        Some(error) => Err(error),
        None => Ok(()),
    }
}

pub(crate) fn best_effort_drain_pending_source_blob_deletions(state: &AppState) {
    if let Err(error) = drain_pending_source_blob_deletions(state) {
        tracing::warn!(?error, "failed to drain pending source blob deletions");
    }
}

#[allow(dead_code)]
pub(crate) fn live_tree(repo: &StoredRepository) -> BTreeMap<ScopePath, SourceBlob> {
    repo.live_tree()
}
