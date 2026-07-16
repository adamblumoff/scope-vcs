use super::{
    policy::{Policy, PolicyError, Principal, PrincipalKind, ScopePath, Visibility},
    projection::{SourceGraph, VisibilityEvent},
    repo_config::{ConfigVisibility, RepoConfig},
    requests::{
        CreditLedgerEntry, Request, RequestDiscussion, RequestDiscussionReadState,
        RequestDiscussionReply, RequestEvent, UserCreditAccount,
    },
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
    pub can_manage_members: bool,
    pub can_delete_repo: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MainPushMode {
    Denied,
    FirstPush,
    Published,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RepositoryPushPolicy {
    pub access: RepositoryAccess,
    pub mode: MainPushMode,
}

impl RepositoryAccess {
    pub fn public() -> Self {
        Self {
            actor: RepositoryActor::Public,
            can_read_private_files: false,
            can_push: false,
            can_change_file_visibility: false,
            can_apply_changes: false,
            can_manage_members: false,
            can_delete_repo: false,
        }
    }
}

pub fn repository_access_for_user_id(
    owner_user_id: &str,
    publication_state: RepoPublicationState,
    member_permissions: Option<RepositoryMemberPermissions>,
    user_id: &str,
) -> RepositoryAccess {
    let published = publication_state == RepoPublicationState::Published;
    if owner_user_id == user_id {
        return RepositoryAccess {
            actor: RepositoryActor::Owner,
            can_read_private_files: true,
            can_push: published,
            can_change_file_visibility: true,
            can_apply_changes: true,
            can_manage_members: published,
            can_delete_repo: true,
        };
    }

    let Some(permissions) = member_permissions else {
        return RepositoryAccess::public();
    };
    RepositoryAccess {
        actor: RepositoryActor::Member,
        can_read_private_files: published,
        can_push: published && permissions.can_push,
        can_change_file_visibility: published && permissions.can_change_file_visibility,
        can_apply_changes: published && permissions.can_apply_changes,
        can_manage_members: false,
        can_delete_repo: false,
    }
}

pub fn repository_push_policy_for_user_id(
    owner_user_id: &str,
    publication_state: RepoPublicationState,
    member_permissions: Option<RepositoryMemberPermissions>,
    user_id: &str,
) -> RepositoryPushPolicy {
    let access = repository_access_for_user_id(
        owner_user_id,
        publication_state,
        member_permissions,
        user_id,
    );
    let mode = if publication_state == RepoPublicationState::Unpublished && owner_user_id == user_id
    {
        MainPushMode::FirstPush
    } else if publication_state == RepoPublicationState::Published && access.can_push {
        MainPushMode::Published
    } else {
        MainPushMode::Denied
    };
    RepositoryPushPolicy { access, mode }
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

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct GitHead {
    pub head_oid: String,
    pub segment_sequence: u64,
    pub change_version: u64,
    pub manifest: SourceBlob,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct GitSegment {
    pub sequence: u64,
    pub base_oid: Option<String>,
    pub head_oid: String,
    pub object: SourceBlob,
    pub manifest: SourceBlob,
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
#[cfg_attr(any(test, feature = "ts"), derive(ts_rs::TS))]
pub enum FileChangeKind {
    Added,
    Modified,
    Deleted,
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
    pub repo_config: RepoConfig,
    pub first_push_token: Option<FirstPushToken>,
    pub git_push_token: Option<GitPushToken>,
    pub policy: Policy,
    pub graph: SourceGraph,
    pub visibility_events: Vec<VisibilityEvent>,
    pub live_files: BTreeMap<ScopePath, SourceBlob>,
    pub git_head: Option<GitHead>,
    pub git_segments: Vec<GitSegment>,
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
            repo_config: RepoConfig::with_default_visibility(config_default),
            first_push_token: None,
            git_push_token: None,
            policy: Policy::new(default_visibility),
            graph: SourceGraph {
                repo_id: id.clone(),
                commits: Vec::new(),
            },
            visibility_events: Vec::new(),
            live_files: BTreeMap::new(),
            git_head: None,
            git_segments: Vec::new(),
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
        repository_access_for_user_id(
            &self.record.owner_user_id,
            self.record.publication_state,
            self.member_for_user(user_id)
                .map(|member| member.permissions),
            user_id,
        )
    }

    pub fn is_waiting_for_first_push(&self) -> bool {
        self.record.publication_state == RepoPublicationState::Unpublished
    }

    pub fn graph_has_file(&self, path: &ScopePath) -> bool {
        self.live_files.contains_key(path)
    }

    pub fn bump_change_version(&mut self) {
        self.record.change_version = self.record.change_version.saturating_add(1);
    }

    pub fn live_tree(&self) -> BTreeMap<ScopePath, SourceBlob> {
        self.live_files.clone()
    }

    pub fn source_blobs(&self) -> Vec<SourceBlob> {
        let mut blobs = Vec::new();
        blobs.extend(self.git_head.iter().map(|head| head.manifest.clone()));
        blobs.extend(
            self.git_segments
                .iter()
                .flat_map(|segment| [segment.object.clone(), segment.manifest.clone()]),
        );
        for change in self.graph.commits.iter().flat_map(|commit| &commit.changes) {
            blobs.extend(change.old_content.clone());
            blobs.extend(change.new_content.clone());
        }
        for event in &self.visibility_events {
            blobs.extend(event.current_content.clone());
        }
        blobs
    }

    pub fn has_file_for_visibility_update(&self, path: &ScopePath) -> bool {
        self.graph_has_file(path)
    }

    pub fn can_read_path(&self, principal: &Principal, path: &ScopePath) -> bool {
        if principal.kind == PrincipalKind::Public {
            return self.record.publication_state == RepoPublicationState::Published
                && self.policy.can_read(path, false);
        }

        let access = self.access_for_principal(principal);
        match access.actor {
            RepositoryActor::Owner => self.policy.can_read(path, true),
            RepositoryActor::Member => {
                self.record.publication_state == RepoPublicationState::Published
                    && self.policy.can_read(path, access.can_read_private_files)
            }
            RepositoryActor::Public => false,
        }
    }

    pub fn can_push(&self, principal: &Principal) -> bool {
        self.access_for_principal(principal).can_push
    }

    pub fn push_policy_for_user_id(&self, user_id: &str) -> RepositoryPushPolicy {
        repository_push_policy_for_user_id(
            &self.record.owner_user_id,
            self.record.publication_state,
            self.member_for_user(user_id)
                .map(|member| member.permissions),
            user_id,
        )
    }

    pub fn is_maintainer_user_id(&self, user_id: &str) -> bool {
        matches!(
            self.access_for_user_id(user_id).actor,
            RepositoryActor::Owner | RepositoryActor::Member
        )
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct AppCatalog {
    pub users: BTreeMap<String, UserAccount>,
    pub repositories: BTreeMap<String, StoredRepository>,
    pub requests: BTreeMap<String, Request>,
    pub request_discussions: BTreeMap<String, RequestDiscussion>,
    pub request_discussion_replies: BTreeMap<String, RequestDiscussionReply>,
    pub request_discussion_read_states: BTreeMap<String, RequestDiscussionReadState>,
    pub request_events: BTreeMap<String, RequestEvent>,
    pub user_credit_accounts: BTreeMap<String, UserCreditAccount>,
    pub credit_ledger_entries: BTreeMap<String, CreditLedgerEntry>,
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

pub fn repo_relative_scope_path(path: &str) -> Result<ScopePath, PolicyError> {
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
