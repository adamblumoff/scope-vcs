use crate::domain::policy::{Principal, ScopePath};
use crate::domain::projection::{
    AuthorVisibility, LogicalCommit, MixedCommitPolicy, project_graph,
};
use crate::domain::store::{
    AppCatalog, RepoPublicationState, RepoRole, SourceBlob, StoredRepository,
};
use crate::{
    auth::shoo::ShooVerifier,
    config::{data_dir, git_repo_root},
    db::MetadataStore,
    error::ApiError,
    http::responses::pending_import_changes,
    object_store::{EncryptedObjectStore, ObjectStore, S3ObjectStore},
    persistence::ensure_private_dir,
};
use std::{collections::BTreeMap, path::PathBuf, sync::Arc};

#[derive(Clone)]
pub struct AppState {
    pub(crate) metadata: MetadataStore,
    pub(crate) data_dir: Arc<PathBuf>,
    pub(crate) shoo: ShooVerifier,
    pub(crate) object_store: Arc<dyn ObjectStore>,
}

impl AppState {
    pub fn from_env() -> anyhow::Result<Self> {
        let repo_root = git_repo_root();
        let data_dir = data_dir(&repo_root);
        ensure_private_dir(&data_dir).map_err(|error| anyhow::anyhow!(error.message))?;

        let state = Self {
            metadata: MetadataStore::connect_from_env()?,
            data_dir: Arc::new(data_dir),
            shoo: ShooVerifier::from_env(),
            object_store: Arc::new(EncryptedObjectStore::from_env(Arc::new(
                S3ObjectStore::from_env()?,
            ))?),
        };
        best_effort_drain_pending_source_blob_deletions(&state);
        Ok(state)
    }

    #[cfg(test)]
    pub(crate) fn test_state() -> Self {
        use crate::{config::SHOO_ISSUER, domain::store::app_catalog, persistence::test_data_dir};

        Self {
            metadata: MetadataStore::memory(app_catalog()),
            data_dir: Arc::new(test_data_dir()),
            shoo: ShooVerifier::new(
                SHOO_ISSUER,
                Some("origin:http://localhost:3000".to_string()),
                "http://127.0.0.1/.well-known/jwks.json",
            ),
            object_store: Arc::new(crate::object_store::MemoryObjectStore::new()),
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
    let owner = owner.to_string();
    let name = name.to_string();
    state.metadata.read(move |catalog| {
        catalog
            .repository(&owner, &name)
            .cloned()
            .ok_or_else(|| ApiError::not_found(format!("repo {owner}/{name} not found")))
    })
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

pub(crate) fn has_visible_projected_files(repo: &StoredRepository, principal: &Principal) -> bool {
    let projection = project_graph(&repo.policy, &repo.graph, principal);
    let mut visible_paths = std::collections::BTreeSet::new();
    for change in projection.commits.iter().flat_map(|commit| &commit.changes) {
        if change.new_content.is_some() {
            visible_paths.insert(change.path.clone());
        } else {
            visible_paths.remove(&change.path);
        }
    }
    !visible_paths.is_empty()
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

pub(crate) fn ensure_pending_publish(repo: &StoredRepository) -> Result<(), ApiError> {
    if repo.record.publication_state != RepoPublicationState::PendingPublish {
        return Err(ApiError::bad_request("repo is not pending publish"));
    }
    if repo.pending_import.is_none() {
        return Err(ApiError::bad_request(
            "repo has no pending import to publish",
        ));
    }
    Ok(())
}

pub(crate) fn promote_pending_import(repo: &mut StoredRepository) -> Result<(), ApiError> {
    ensure_pending_publish(repo)?;
    let pending = repo
        .pending_import
        .take()
        .ok_or_else(|| ApiError::bad_request("repo has no pending import to publish"))?;
    let changes = pending_import_changes(&pending);
    let parent_ids = repo
        .graph
        .commits
        .last()
        .map(|commit| vec![commit.id.clone()])
        .unwrap_or_default();
    let logical_id = format!(
        "rv_git_{}",
        pending
            .head_oid
            .get(..12)
            .unwrap_or(pending.head_oid.as_str())
    );
    repo.graph.commits.push(LogicalCommit {
        id: logical_id,
        parent_ids,
        author_id: repo.record.owner_user_id.clone(),
        author_visibility: AuthorVisibility::Visible,
        message: format!("Import pushed {}", pending.default_branch),
        mixed_policy: MixedCommitPolicy::SyntheticPublicCommit,
        changes,
    });
    repo.git_snapshot = Some(pending.git_snapshot);
    repo.record.publication_state = RepoPublicationState::Published;
    Ok(())
}

pub(crate) fn role_for_principal(
    state: &AppState,
    repo: &StoredRepository,
    principal: &Principal,
) -> Result<Option<RepoRole>, ApiError> {
    let repo = repo.clone();
    let principal = principal.clone();
    state
        .metadata
        .read(move |catalog| Ok(catalog.role_for_principal(&repo, &principal)))
}

pub(crate) fn can_read_path(
    state: &AppState,
    repo: &StoredRepository,
    principal: &Principal,
    path: &ScopePath,
) -> Result<bool, ApiError> {
    let repo = repo.clone();
    let principal = principal.clone();
    let path = path.clone();
    state
        .metadata
        .read(move |catalog| Ok(catalog.can_read_path(&repo, &principal, &path)))
}

pub(crate) fn can_write_path(
    state: &AppState,
    repo: &StoredRepository,
    principal: &Principal,
    path: &ScopePath,
) -> Result<bool, ApiError> {
    let repo = repo.clone();
    let principal = principal.clone();
    let path = path.clone();
    state
        .metadata
        .read(move |catalog| Ok(catalog.can_write_path(&repo, &principal, &path)))
}

pub(crate) fn graph_has_file(repo: &StoredRepository, path: &ScopePath) -> bool {
    let mut present = false;
    for change in repo.graph.commits.iter().flat_map(|commit| &commit.changes) {
        if change.path.as_str() == path.as_str() {
            present = change.new_content.is_some();
        }
    }

    present
}

pub(crate) fn repo_source_blobs(repo: &StoredRepository) -> Vec<SourceBlob> {
    let mut blobs = Vec::new();
    if let Some(pending) = &repo.pending_import {
        blobs.push(pending.git_snapshot.clone());
        blobs.extend(pending.files.iter().map(|file| file.blob.clone()));
    }
    blobs.extend(repo.git_snapshot.clone());
    for change in repo.graph.commits.iter().flat_map(|commit| &commit.changes) {
        blobs.extend(change.old_content.clone());
        blobs.extend(change.new_content.clone());
    }
    if let Some(staged) = &repo.staged_update {
        blobs.push(staged.git_snapshot.clone());
        for change in &staged.changes {
            blobs.extend(change.old_content.clone());
            blobs.extend(change.new_content.clone());
        }
    }
    blobs
}

pub(crate) fn delete_unreferenced_source_blobs(
    state: &AppState,
    blobs: &[SourceBlob],
) -> Result<(), ApiError> {
    let cleanup_state = state.clone();
    let blobs = blobs.to_vec();
    state.metadata.update(move |catalog| {
        delete_unreferenced_source_blobs_against_catalog(&cleanup_state, catalog, &blobs)
    })
}

pub(crate) fn delete_unreferenced_source_blobs_against_catalog(
    state: &AppState,
    catalog: &AppCatalog,
    blobs: &[SourceBlob],
) -> Result<(), ApiError> {
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
    for blob in unreferenced.values() {
        crate::git::storage::delete_raw_git_snapshot_cache(state, blob)?;
        state.object_store.delete(&blob.object_key)?;
    }
    Ok(())
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

pub(crate) fn drain_pending_source_blob_deletions(state: &AppState) -> Result<(), ApiError> {
    let cleanup_state = state.clone();
    state.metadata.update(move |catalog| {
        let pending = catalog.pending_source_blob_deletions.clone();
        if pending.is_empty() {
            return Ok(());
        }

        delete_unreferenced_source_blobs_against_catalog(&cleanup_state, catalog, &pending)?;

        let deleted_keys = pending
            .iter()
            .map(|blob| blob.object_key.clone())
            .collect::<std::collections::BTreeSet<_>>();
        catalog
            .pending_source_blob_deletions
            .retain(|blob| !deleted_keys.contains(&blob.object_key));
        Ok(())
    })
}

pub(crate) fn best_effort_drain_pending_source_blob_deletions(state: &AppState) {
    if let Err(error) = drain_pending_source_blob_deletions(state) {
        tracing::warn!(?error, "failed to drain pending source blob deletions");
    }
}

#[allow(dead_code)]
pub(crate) fn live_tree(repo: &StoredRepository) -> BTreeMap<ScopePath, SourceBlob> {
    let mut tree = BTreeMap::new();
    for change in repo.graph.commits.iter().flat_map(|commit| &commit.changes) {
        match &change.new_content {
            Some(blob) => {
                tree.insert(change.path.clone(), blob.clone());
            }
            None => {
                tree.remove(&change.path);
            }
        }
    }
    tree
}
