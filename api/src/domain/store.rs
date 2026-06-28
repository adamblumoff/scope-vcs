use super::{
    policy::{Policy, PolicyError, Principal, PrincipalKind, ScopePath, Visibility},
    projection::{SourceGraph, VisibilityEvent},
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum AccountAccess {
    Public,
    Member,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct UserAccount {
    pub id: String,
    pub handle: String,
    pub email: String,
    pub email_verified: bool,
    pub access: AccountAccess,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub enum RepoRole {
    Reader,
    Writer,
    Maintainer,
    Owner,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub enum RepoPublicationState {
    PendingFirstPush,
    PendingPublish,
    Published,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub enum FirstPushTokenStatus {
    Active,
    Expired,
    Used,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct FirstPushToken {
    pub token_hash: String,
    pub secret: Option<String>,
    pub owner_user_id: String,
    pub created_at_unix: u64,
    pub expires_at_unix: u64,
    pub used_at_unix: Option<u64>,
}

impl FirstPushToken {
    pub fn status_at(&self, now_unix: u64) -> FirstPushTokenStatus {
        if self.used_at_unix.is_some() {
            FirstPushTokenStatus::Used
        } else if now_unix >= self.expires_at_unix {
            FirstPushTokenStatus::Expired
        } else {
            FirstPushTokenStatus::Active
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct GitPushToken {
    pub token_hash: String,
    pub owner_user_id: String,
    pub created_at_unix: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct GitCloneToken {
    pub token_hash: String,
    pub user_id: String,
    pub created_at_unix: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SourceBlob {
    pub object_key: String,
    pub sha256: String,
    pub git_oid: String,
    pub size_bytes: u64,
    pub line_count: usize,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct LineDiff {
    pub additions: usize,
    pub deletions: usize,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RepoStorageCleanup {
    pub owner_handle: String,
    pub repo_name: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PendingImportFile {
    pub path: String,
    pub mode: String,
    pub oid: String,
    pub blob: SourceBlob,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PendingImport {
    pub default_branch: String,
    pub head_oid: String,
    pub tree_oid: String,
    pub imported_at_unix: u64,
    pub git_snapshot: SourceBlob,
    pub files: Vec<PendingImportFile>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RepoRecord {
    pub id: String,
    pub owner_handle: String,
    pub name: String,
    pub owner_user_id: String,
    pub publication_state: RepoPublicationState,
    pub default_visibility: Visibility,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RepoSettings {
    pub include_ignored_files: bool,
    pub review_pushes_before_applying: bool,
}

impl Default for RepoSettings {
    fn default() -> Self {
        Self {
            include_ignored_files: false,
            review_pushes_before_applying: true,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub enum StagedFileChangeKind {
    Added,
    Modified,
    Deleted,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct StagedFileChange {
    pub path: ScopePath,
    pub old_content: Option<SourceBlob>,
    pub new_content: Option<SourceBlob>,
    pub line_diff: LineDiff,
    pub visibility: Visibility,
    pub kind: StagedFileChangeKind,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct StagedRepoUpdate {
    pub id: String,
    pub branch: String,
    pub base_live_commit_id: Option<String>,
    pub author_id: String,
    pub message: String,
    pub git_snapshot: SourceBlob,
    pub changes: Vec<StagedFileChange>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RepoMembership {
    pub repo_id: String,
    pub user_id: String,
    pub role: RepoRole,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum InvitationState {
    Pending,
    Accepted,
    Revoked,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RepoInvitation {
    pub id: String,
    pub repo_id: String,
    pub invited_email: String,
    pub role: RepoRole,
    pub invited_by_user_id: String,
    pub state: InvitationState,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct StoredRepository {
    pub record: RepoRecord,
    pub settings: RepoSettings,
    pub first_push_token: Option<FirstPushToken>,
    pub git_push_token: Option<GitPushToken>,
    pub git_clone_tokens: Vec<GitCloneToken>,
    pub pending_import: Option<PendingImport>,
    pub policy: Policy,
    pub graph: SourceGraph,
    pub visibility_events: Vec<VisibilityEvent>,
    pub git_snapshot: Option<SourceBlob>,
    pub staged_update: Option<StagedRepoUpdate>,
    pub memberships: Vec<RepoMembership>,
    pub invitations: Vec<RepoInvitation>,
}

impl StoredRepository {
    pub fn new(
        owner: &UserAccount,
        name: &str,
        default_visibility: Visibility,
    ) -> Result<Self, CatalogError> {
        let name = validate_repo_name(name)?;
        let id = repo_id(&owner.handle, &name);
        Ok(Self {
            record: RepoRecord {
                id: id.clone(),
                owner_handle: owner.handle.clone(),
                name,
                owner_user_id: owner.id.clone(),
                publication_state: RepoPublicationState::PendingFirstPush,
                default_visibility,
            },
            settings: RepoSettings::default(),
            first_push_token: None,
            git_push_token: None,
            git_clone_tokens: Vec::new(),
            pending_import: None,
            policy: Policy::new(default_visibility, owner.id.clone()),
            graph: SourceGraph {
                repo_id: id.clone(),
                commits: Vec::new(),
            },
            visibility_events: Vec::new(),
            git_snapshot: None,
            staged_update: None,
            memberships: vec![RepoMembership {
                repo_id: id,
                user_id: owner.id.clone(),
                role: RepoRole::Owner,
            }],
            invitations: Vec::new(),
        })
    }

    pub fn owner_ids(&self) -> Vec<String> {
        let mut owner_ids = self
            .memberships
            .iter()
            .filter(|membership| membership.role == RepoRole::Owner)
            .map(|membership| membership.user_id.clone())
            .collect::<Vec<_>>();
        if !owner_ids.contains(&self.record.owner_user_id) {
            owner_ids.push(self.record.owner_user_id.clone());
        }
        owner_ids.sort();
        owner_ids.dedup();
        owner_ids
    }

    pub fn graph_has_file(&self, path: &ScopePath) -> bool {
        let mut present = false;
        for change in self.graph.commits.iter().flat_map(|commit| &commit.changes) {
            if change.path.as_str() == path.as_str() {
                present = change.new_content.is_some();
            }
        }
        present
    }

    pub fn live_tree(&self) -> BTreeMap<ScopePath, SourceBlob> {
        let mut tree = BTreeMap::new();
        for change in self.graph.commits.iter().flat_map(|commit| &commit.changes) {
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

    pub fn source_blobs(&self) -> Vec<SourceBlob> {
        let mut blobs = Vec::new();
        if let Some(pending) = &self.pending_import {
            blobs.push(pending.git_snapshot.clone());
            blobs.extend(pending.files.iter().map(|file| file.blob.clone()));
        }
        blobs.extend(self.git_snapshot.clone());
        for change in self.graph.commits.iter().flat_map(|commit| &commit.changes) {
            blobs.extend(change.old_content.clone());
            blobs.extend(change.new_content.clone());
        }
        for event in &self.visibility_events {
            blobs.extend(event.current_content.clone());
        }
        if let Some(staged) = &self.staged_update {
            blobs.push(staged.git_snapshot.clone());
            for change in &staged.changes {
                blobs.extend(change.old_content.clone());
                blobs.extend(change.new_content.clone());
            }
        }
        blobs
    }

    pub fn has_file_for_visibility_update(&self, path: &ScopePath) -> bool {
        if self.record.publication_state == RepoPublicationState::PendingPublish {
            self.pending_import_has_file(path)
        } else {
            self.graph_has_file(path)
        }
    }

    fn pending_import_has_file(&self, path: &ScopePath) -> bool {
        self.pending_import.as_ref().is_some_and(|pending| {
            pending.files.iter().any(|file| {
                pending_import_scope_path(&file.path)
                    .map(|pending_path| pending_path.as_str() == path.as_str())
                    .unwrap_or(false)
            })
        })
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct AppCatalog {
    pub users: BTreeMap<String, UserAccount>,
    pub repositories: BTreeMap<String, StoredRepository>,
    pub pending_repo_storage_deletions: Vec<RepoStorageCleanup>,
    pub pending_source_blob_deletions: Vec<SourceBlob>,
}

impl AppCatalog {
    pub fn repository(&self, owner: &str, name: &str) -> Option<&StoredRepository> {
        self.repositories.get(&repo_id(owner, name))
    }

    pub fn repositories_for_user(&self, user_id: &str) -> Vec<&StoredRepository> {
        self.repositories
            .values()
            .filter(|repo| {
                repo.memberships
                    .iter()
                    .any(|membership| membership.user_id == user_id)
            })
            .collect()
    }

    pub fn create_repository(
        &mut self,
        owner: &UserAccount,
        name: &str,
        default_visibility: Visibility,
    ) -> Result<&StoredRepository, CatalogError> {
        let repository = StoredRepository::new(owner, name, default_visibility)?;
        let id = repository.record.id.clone();
        if self.repositories.contains_key(&id) {
            return Err(CatalogError::RepositoryExists(id));
        }
        self.repositories.insert(id.clone(), repository);
        Ok(self.repositories.get(&id).expect("repository was inserted"))
    }

    pub fn role_for_principal(
        &self,
        repo: &StoredRepository,
        principal: &Principal,
    ) -> Option<RepoRole> {
        if principal.kind != PrincipalKind::Public && principal.id == repo.record.owner_user_id {
            return Some(RepoRole::Owner);
        }

        repo.memberships
            .iter()
            .find(|membership| membership.user_id == principal.id)
            .map(|membership| membership.role)
    }

    pub fn can_read_path(
        &self,
        repo: &StoredRepository,
        principal: &Principal,
        path: &ScopePath,
    ) -> bool {
        if principal.kind == PrincipalKind::Public {
            return repo.record.publication_state == RepoPublicationState::Published
                && repo.policy.can_read(principal, path);
        }

        let Some(role) = self.role_for_principal(repo, principal) else {
            return false;
        };

        let lifecycle_allows_read = repo.record.publication_state
            == RepoPublicationState::Published
            || role == RepoRole::Owner;

        lifecycle_allows_read && repo.policy.can_read(principal, path)
    }

    pub fn can_write_path(
        &self,
        repo: &StoredRepository,
        principal: &Principal,
        path: &ScopePath,
    ) -> bool {
        self.role_for_principal(repo, principal)
            .is_some_and(|role| role >= RepoRole::Writer)
            && self.can_read_path(repo, principal, path)
    }
}

pub fn app_catalog() -> AppCatalog {
    AppCatalog::default()
}

pub fn repo_id(owner: &str, name: &str) -> String {
    format!(
        "{}/{}",
        owner.trim().to_ascii_lowercase(),
        name.trim().to_ascii_lowercase()
    )
}

pub fn pending_import_scope_path(path: &str) -> Result<ScopePath, PolicyError> {
    let path = if path.starts_with('/') {
        path.to_string()
    } else {
        format!("/{path}")
    };
    ScopePath::parse(path)
}

fn validate_repo_name(name: &str) -> Result<String, CatalogError> {
    let name = name.trim().to_ascii_lowercase();
    if name.is_empty() {
        return Err(CatalogError::InvalidRepositoryName(
            "repository name is required".to_string(),
        ));
    }
    if name == "." || name == ".." {
        return Err(CatalogError::InvalidRepositoryName(
            "repository name cannot be . or ..".to_string(),
        ));
    }
    if name.len() > 80 {
        return Err(CatalogError::InvalidRepositoryName(
            "repository name must be 80 characters or fewer".to_string(),
        ));
    }
    if !name
        .bytes()
        .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'-' | b'_'))
    {
        return Err(CatalogError::InvalidRepositoryName(
            "repository name can only use letters, numbers, dots, dashes, or underscores"
                .to_string(),
        ));
    }

    Ok(name)
}

#[derive(Debug, thiserror::Error)]
pub enum CatalogError {
    #[error("{0}")]
    InvalidRepositoryName(String),
    #[error("repo {0} already exists")]
    RepositoryExists(String),
}
