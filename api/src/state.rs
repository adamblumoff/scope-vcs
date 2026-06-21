use crate::domain::git_projection::build_virtual_git_projection;
use crate::domain::policy::{Principal, ScopePath};
use crate::domain::projection::{
    AuthorVisibility, LogicalCommit, MixedCommitPolicy, project_graph,
};
use crate::domain::store::{RepoPublicationState, RepoRole, StoredRepository};
use crate::{
    auth::shoo::ShooVerifier,
    config::{data_dir, git_repo_root},
    db::MetadataStore,
    error::ApiError,
    http::responses::pending_import_changes,
    persistence::ensure_private_dir,
};
use std::{collections::BTreeMap, path::PathBuf, sync::Arc};

#[derive(Clone)]
pub struct AppState {
    pub(crate) metadata: MetadataStore,
    pub(crate) data_dir: Arc<PathBuf>,
    pub(crate) shoo: ShooVerifier,
}

impl AppState {
    pub async fn from_env() -> anyhow::Result<Self> {
        let repo_root = git_repo_root();
        let data_dir = data_dir(&repo_root);
        ensure_private_dir(&data_dir).map_err(|error| anyhow::anyhow!(error.message))?;

        Ok(Self {
            metadata: MetadataStore::connect_from_env()?,
            data_dir: Arc::new(data_dir),
            shoo: ShooVerifier::from_env(),
        })
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
    build_virtual_git_projection(&projection).blobs.is_empty() == false
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
    let changes = pending_import_changes(&pending)?;
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

#[allow(dead_code)]
pub(crate) fn live_tree(repo: &StoredRepository) -> BTreeMap<ScopePath, String> {
    let mut tree = BTreeMap::new();
    for change in repo.graph.commits.iter().flat_map(|commit| &commit.changes) {
        match &change.new_content {
            Some(content) => {
                tree.insert(change.path.clone(), content.clone());
            }
            None => {
                tree.remove(&change.path);
            }
        }
    }
    tree
}
