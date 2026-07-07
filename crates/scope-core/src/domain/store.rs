use super::{
    policy::{Policy, PolicyError, Principal, PrincipalKind, ScopePath, Visibility},
    projection::{SourceGraph, VisibilityEvent},
    repo_config::{ConfigVisibility, RepoConfig},
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct UserAccount {
    pub id: String,
    pub handle: String,
    pub email: String,
    pub email_verified: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(any(test, feature = "ts"), derive(ts_rs::TS))]
pub enum RepositoryActor {
    Public,
    Member,
    Owner,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(any(test, feature = "ts"), derive(ts_rs::TS))]
pub enum RepoPublicationState {
    Unpublished,
    Published,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(any(test, feature = "ts"), derive(ts_rs::TS))]
pub struct RepositoryMemberPermissions {
    pub can_push: bool,
    pub can_change_file_visibility: bool,
    pub can_apply_changes: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RepositoryAccess {
    pub actor: RepositoryActor,
    pub can_read_private_files: bool,
    pub can_push: bool,
    pub can_change_file_visibility: bool,
    pub can_apply_changes: bool,
    pub can_update_repo_settings: bool,
    pub can_manage_members: bool,
    pub can_delete_repo: bool,
}

impl RepositoryAccess {
    pub fn public() -> Self {
        Self {
            actor: RepositoryActor::Public,
            can_read_private_files: false,
            can_push: false,
            can_change_file_visibility: false,
            can_apply_changes: false,
            can_update_repo_settings: false,
            can_manage_members: false,
            can_delete_repo: false,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(any(test, feature = "ts"), derive(ts_rs::TS))]
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
pub struct SourceBlob {
    pub object_key: String,
    pub sha256: String,
    pub git_oid: String,
    pub git_file_mode: String,
    pub size_bytes: u64,
}

pub const DEFAULT_GIT_FILE_MODE: &str = "100644";
pub const EXECUTABLE_GIT_FILE_MODE: &str = "100755";

pub fn is_supported_git_file_mode(mode: &str) -> bool {
    matches!(mode, DEFAULT_GIT_FILE_MODE | EXECUTABLE_GIT_FILE_MODE)
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
    pub change_version: u64,
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
#[cfg_attr(any(test, feature = "ts"), derive(ts_rs::TS))]
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
pub struct RepositoryMember {
    pub repo_id: String,
    pub user_id: String,
    pub permissions: RepositoryMemberPermissions,
    pub created_at_unix: u64,
    pub updated_at_unix: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(any(test, feature = "ts"), derive(ts_rs::TS))]
pub enum RepositoryInviteState {
    Pending,
    Accepted,
    Revoked,
    Expired,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RepositoryInvite {
    pub id: String,
    pub repo_id: String,
    pub invited_email: String,
    pub invited_email_normalized: String,
    pub permissions: RepositoryMemberPermissions,
    pub invited_by_user_id: String,
    pub state: RepositoryInviteState,
    pub token_hash: String,
    pub created_at_unix: u64,
    pub updated_at_unix: u64,
    pub expires_at_unix: u64,
    pub accepted_by_user_id: Option<String>,
    pub accepted_at_unix: Option<u64>,
    pub revoked_at_unix: Option<u64>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct StoredRepository {
    pub record: RepoRecord,
    pub settings: RepoSettings,
    pub repo_config: RepoConfig,
    pub first_push_token: Option<FirstPushToken>,
    pub git_push_token: Option<GitPushToken>,
    pub pending_import: Option<PendingImport>,
    pub policy: Policy,
    pub graph: SourceGraph,
    pub visibility_events: Vec<VisibilityEvent>,
    pub git_snapshot: Option<SourceBlob>,
    pub staged_update: Option<StagedRepoUpdate>,
    pub members: Vec<RepositoryMember>,
    pub invitations: Vec<RepositoryInvite>,
}

impl StoredRepository {
    pub fn new(
        owner: &UserAccount,
        name: &str,
        default_visibility: Visibility,
    ) -> Result<Self, CatalogError> {
        let name = validate_repo_name(name)?;
        let id = repo_id(&owner.handle, &name);
        let config_default = ConfigVisibility::from(default_visibility);
        Ok(Self {
            record: RepoRecord {
                id: id.clone(),
                owner_handle: owner.handle.clone(),
                name,
                owner_user_id: owner.id.clone(),
                publication_state: RepoPublicationState::Unpublished,
                default_visibility,
                change_version: 1,
            },
            settings: RepoSettings::default(),
            repo_config: RepoConfig::with_default_visibility(config_default),
            first_push_token: None,
            git_push_token: None,
            pending_import: None,
            policy: Policy::new(default_visibility),
            graph: SourceGraph {
                repo_id: id.clone(),
                commits: Vec::new(),
            },
            visibility_events: Vec::new(),
            git_snapshot: None,
            staged_update: None,
            members: Vec::new(),
            invitations: Vec::new(),
        })
    }

    pub fn is_owner_user(&self, user_id: &str) -> bool {
        self.record.owner_user_id == user_id
    }

    pub fn member_for_user(&self, user_id: &str) -> Option<&RepositoryMember> {
        self.members.iter().find(|member| member.user_id == user_id)
    }

    pub fn access_for_principal(&self, principal: &Principal) -> RepositoryAccess {
        if principal.kind == PrincipalKind::Public {
            return RepositoryAccess::public();
        }

        self.access_for_user_id(&principal.id)
    }

    pub fn access_for_user_id(&self, user_id: &str) -> RepositoryAccess {
        let published = self.record.publication_state == RepoPublicationState::Published;
        if self.is_owner_user(user_id) {
            return RepositoryAccess {
                actor: RepositoryActor::Owner,
                can_read_private_files: true,
                can_push: published,
                can_change_file_visibility: true,
                can_apply_changes: true,
                can_update_repo_settings: true,
                can_manage_members: published,
                can_delete_repo: true,
            };
        }

        let Some(member) = self.member_for_user(user_id) else {
            return RepositoryAccess::public();
        };
        let permissions = member.permissions;
        RepositoryAccess {
            actor: RepositoryActor::Member,
            can_read_private_files: published,
            can_push: published && permissions.can_push,
            can_change_file_visibility: published && permissions.can_change_file_visibility,
            can_apply_changes: published && permissions.can_apply_changes,
            can_update_repo_settings: false,
            can_manage_members: false,
            can_delete_repo: false,
        }
    }

    pub fn is_waiting_for_first_push(&self) -> bool {
        self.record.publication_state == RepoPublicationState::Unpublished
            && self.pending_import.is_none()
    }

    pub fn has_pending_import_review(&self) -> bool {
        self.record.publication_state == RepoPublicationState::Unpublished
            && self.pending_import.is_some()
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

    pub fn bump_change_version(&mut self) {
        self.record.change_version = self.record.change_version.saturating_add(1);
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
        if self.has_pending_import_review() {
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
                repo.record.owner_user_id == user_id
                    || repo.members.iter().any(|member| member.user_id == user_id)
            })
            .collect()
    }

    pub fn can_read_path(
        &self,
        repo: &StoredRepository,
        principal: &Principal,
        path: &ScopePath,
    ) -> bool {
        if principal.kind == PrincipalKind::Public {
            return repo.record.publication_state == RepoPublicationState::Published
                && repo.policy.can_read(path, false);
        }

        let access = repo.access_for_principal(principal);
        match access.actor {
            RepositoryActor::Owner => repo.policy.can_read(path, true),
            RepositoryActor::Member => {
                repo.record.publication_state == RepoPublicationState::Published
                    && repo.policy.can_read(path, access.can_read_private_files)
            }
            RepositoryActor::Public => false,
        }
    }

    pub fn can_push(&self, repo: &StoredRepository, principal: &Principal) -> bool {
        repo.access_for_principal(principal).can_push
    }
}

pub fn normalize_repository_invite_email(email: &str) -> String {
    email.trim().to_ascii_lowercase()
}

pub fn repository_member_sort_key(member: &RepositoryMember) -> (&str, &str) {
    (&member.repo_id, &member.user_id)
}

pub fn repository_invite_sort_key(invite: &RepositoryInvite) -> (&str, &str, &str) {
    (
        &invite.repo_id,
        &invite.invited_email_normalized,
        &invite.id,
    )
}
impl AppCatalog {
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
