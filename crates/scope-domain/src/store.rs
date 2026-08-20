use super::{
    policy::{Policy, PolicyError, Principal, PrincipalKind, ScopePath, Visibility},
    projection::{SourceGraph, VisibilityEvent},
    repo_config::{ConfigVisibility, RepoConfig},
};
use crate::content_ref::ContentRef;
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
pub enum RepositoryActor {
    Public,
    Member,
    Owner,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum RepoLifecycleState {
    AwaitingFirstPush,
    Ready,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
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
    Ready,
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
    lifecycle_state: RepoLifecycleState,
    member_permissions: Option<RepositoryMemberPermissions>,
    user_id: &str,
) -> RepositoryAccess {
    let ready = lifecycle_state == RepoLifecycleState::Ready;
    if owner_user_id == user_id {
        return RepositoryAccess {
            actor: RepositoryActor::Owner,
            can_read_private_files: true,
            can_push: ready,
            can_change_file_visibility: true,
            can_apply_changes: true,
            can_manage_members: ready,
            can_delete_repo: true,
        };
    }

    let Some(permissions) = member_permissions else {
        return RepositoryAccess::public();
    };
    RepositoryAccess {
        actor: RepositoryActor::Member,
        can_read_private_files: ready,
        can_push: ready && permissions.can_push,
        can_change_file_visibility: ready && permissions.can_change_file_visibility,
        can_apply_changes: ready && permissions.can_apply_changes,
        can_manage_members: false,
        can_delete_repo: false,
    }
}

pub fn repository_push_policy_for_user_id(
    owner_user_id: &str,
    lifecycle_state: RepoLifecycleState,
    member_permissions: Option<RepositoryMemberPermissions>,
    user_id: &str,
) -> RepositoryPushPolicy {
    let access =
        repository_access_for_user_id(owner_user_id, lifecycle_state, member_permissions, user_id);
    let mode =
        if lifecycle_state == RepoLifecycleState::AwaitingFirstPush && owner_user_id == user_id {
            MainPushMode::FirstPush
        } else if lifecycle_state == RepoLifecycleState::Ready && access.can_push {
            MainPushMode::Ready
        } else {
            MainPushMode::Denied
        };
    RepositoryPushPolicy { access, mode }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
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
    pub content_ref: ContentRef,
    pub sha256: String,
    pub git_oid: String,
    pub git_file_mode: String,
    pub size_bytes: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct GitHead {
    pub head_oid: String,
    pub push_sequence: u64,
    pub change_version: u64,
    pub manifest: SourceBlob,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct GitPackSpan {
    pub first_sequence: u64,
    pub last_sequence: u64,
    pub geometric_tier: u32,
    pub base_oid: Option<String>,
    pub head_oid: String,
    pub object: SourceBlob,
}

impl GitPackSpan {
    pub fn sequence_count(&self) -> Result<u64, GitPackLayoutError> {
        self.last_sequence
            .checked_sub(self.first_sequence)
            .and_then(|distance| distance.checked_add(1))
            .ok_or(GitPackLayoutError::InvalidRange {
                first_sequence: self.first_sequence,
                last_sequence: self.last_sequence,
            })
    }

    pub fn expected_geometric_tier(&self) -> Result<u32, GitPackLayoutError> {
        let sequence_count = self.sequence_count()?;
        if !sequence_count.is_power_of_two() {
            return Err(GitPackLayoutError::NonGeometricCoverage {
                first_sequence: self.first_sequence,
                last_sequence: self.last_sequence,
                sequence_count,
            });
        }
        Ok(sequence_count.ilog2())
    }
}

#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum GitPackLayoutError {
    #[error("Git pack span range {first_sequence}..{last_sequence} is invalid")]
    InvalidRange {
        first_sequence: u64,
        last_sequence: u64,
    },
    #[error(
        "Git pack span {first_sequence}..{last_sequence} has geometric tier {actual}, expected {expected}"
    )]
    InvalidGeometricTier {
        first_sequence: u64,
        last_sequence: u64,
        actual: u32,
        expected: u32,
    },
    #[error(
        "Git pack span {first_sequence}..{last_sequence} covers {sequence_count} pushes instead of a power of two"
    )]
    NonGeometricCoverage {
        first_sequence: u64,
        last_sequence: u64,
        sequence_count: u64,
    },
    #[error("Git pack layout must start at sequence 1, found {first_sequence}")]
    InvalidStart { first_sequence: u64 },
    #[error("Git pack layout starts with a non-empty base object {base_oid}")]
    InvalidStartBase { base_oid: String },
    #[error(
        "Git pack layout has a gap or overlap between sequences {previous_last_sequence} and {next_first_sequence}"
    )]
    NonContiguous {
        previous_last_sequence: u64,
        next_first_sequence: u64,
    },
    #[error(
        "Git pack span history is disconnected: previous head {previous_head_oid}, next base {next_base_oid:?}"
    )]
    DisconnectedHistory {
        previous_head_oid: String,
        next_base_oid: Option<String>,
    },
    #[error(
        "Git pack span tiers must be non-increasing from oldest to newest, found {previous_tier} before {next_tier}"
    )]
    IncreasingTier { previous_tier: u32, next_tier: u32 },
}

pub fn validate_git_pack_layout(spans: &[GitPackSpan]) -> Result<(), GitPackLayoutError> {
    let Some(first) = spans.first() else {
        return Ok(());
    };
    if first.first_sequence != 1 {
        return Err(GitPackLayoutError::InvalidStart {
            first_sequence: first.first_sequence,
        });
    }
    if let Some(base_oid) = &first.base_oid {
        return Err(GitPackLayoutError::InvalidStartBase {
            base_oid: base_oid.clone(),
        });
    }

    validate_git_pack_span_run(spans)
}

pub fn validate_git_pack_span_run(spans: &[GitPackSpan]) -> Result<(), GitPackLayoutError> {
    for (index, span) in spans.iter().enumerate() {
        let expected = span.expected_geometric_tier()?;
        if span.geometric_tier != expected {
            return Err(GitPackLayoutError::InvalidGeometricTier {
                first_sequence: span.first_sequence,
                last_sequence: span.last_sequence,
                actual: span.geometric_tier,
                expected,
            });
        }
        if let Some(previous) = index.checked_sub(1).and_then(|index| spans.get(index)) {
            let expected_first =
                previous
                    .last_sequence
                    .checked_add(1)
                    .ok_or(GitPackLayoutError::InvalidRange {
                        first_sequence: previous.first_sequence,
                        last_sequence: previous.last_sequence,
                    })?;
            if span.first_sequence != expected_first {
                return Err(GitPackLayoutError::NonContiguous {
                    previous_last_sequence: previous.last_sequence,
                    next_first_sequence: span.first_sequence,
                });
            }
            if span.base_oid.as_deref() != Some(previous.head_oid.as_str()) {
                return Err(GitPackLayoutError::DisconnectedHistory {
                    previous_head_oid: previous.head_oid.clone(),
                    next_base_oid: span.base_oid.clone(),
                });
            }
            if span.geometric_tier > previous.geometric_tier {
                return Err(GitPackLayoutError::IncreasingTier {
                    previous_tier: previous.geometric_tier,
                    next_tier: span.geometric_tier,
                });
            }
        }
    }
    Ok(())
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct NativePublicCommit {
    pub oid: String,
    pub parent_oids: Vec<String>,
    pub tree_oid: String,
    pub changed_paths: Vec<ScopePath>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum RequestMergeOrigin {
    Private {
        request_id: String,
        request_head_oid: String,
    },
    Public {
        request_id: String,
        public_base_oid: String,
        public_parent_oids: Vec<String>,
        request_head_oid: String,
        commits: Vec<NativePublicCommit>,
    },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum LogicalCommitOrigin {
    CanonicalPush {
        source_head_oid: String,
    },
    PrivateRequestMerge {
        request_id: String,
        request_head_oid: String,
    },
    PublicRequestMerge {
        request_id: String,
        public_base_oid: String,
        public_parent_oids: Vec<String>,
        request_head_oid: String,
        commits: Vec<NativePublicCommit>,
        preserve_public_commits: bool,
    },
}

impl RequestMergeOrigin {
    pub fn into_logical_origin(self) -> LogicalCommitOrigin {
        match self {
            Self::Private {
                request_id,
                request_head_oid,
            } => LogicalCommitOrigin::PrivateRequestMerge {
                request_id,
                request_head_oid,
            },
            Self::Public {
                request_id,
                public_base_oid,
                public_parent_oids,
                request_head_oid,
                commits,
            } => LogicalCommitOrigin::PublicRequestMerge {
                request_id,
                public_base_oid,
                public_parent_oids,
                request_head_oid,
                commits,
                preserve_public_commits: true,
            },
        }
    }
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
    pub lifecycle_state: RepoLifecycleState,
    pub change_version: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
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
    pub git_pack_spans: Vec<GitPackSpan>,
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
                lifecycle_state: RepoLifecycleState::AwaitingFirstPush,
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
            git_pack_spans: Vec::new(),
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
            self.record.lifecycle_state,
            self.member_for_user(user_id)
                .map(|member| member.permissions),
            user_id,
        )
    }

    pub fn is_waiting_for_first_push(&self) -> bool {
        self.record.lifecycle_state == RepoLifecycleState::AwaitingFirstPush
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
        blobs.extend(self.git_pack_spans.iter().map(|span| span.object.clone()));
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
            return self.record.lifecycle_state == RepoLifecycleState::Ready
                && self.policy.can_read(path, false);
        }

        let access = self.access_for_principal(principal);
        match access.actor {
            RepositoryActor::Owner => self.policy.can_read(path, true),
            RepositoryActor::Member => {
                self.record.lifecycle_state == RepoLifecycleState::Ready
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
            self.record.lifecycle_state,
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
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pack_span(first_sequence: u64, last_sequence: u64, geometric_tier: u32) -> GitPackSpan {
        GitPackSpan {
            first_sequence,
            last_sequence,
            geometric_tier,
            base_oid: (first_sequence > 1).then(|| format!("head-{}", first_sequence - 1)),
            head_oid: format!("head-{last_sequence}"),
            object: SourceBlob {
                content_ref: ContentRef::git_segment_sha256(format!("pack-{first_sequence}")),
                sha256: format!("pack-{first_sequence}"),
                git_oid: format!("head-{last_sequence}"),
                git_file_mode: DEFAULT_GIT_FILE_MODE.to_string(),
                size_bytes: 1,
            },
        }
    }

    #[test]
    fn git_pack_layout_accepts_contiguous_geometric_spans() {
        let spans = [
            pack_span(1, 8, 3),
            pack_span(9, 12, 2),
            pack_span(13, 13, 0),
        ];

        validate_git_pack_layout(&spans).unwrap();
    }

    #[test]
    fn git_pack_layout_rejects_gaps_and_overlaps() {
        let gap = [pack_span(1, 4, 2), pack_span(6, 6, 0)];
        assert_eq!(
            validate_git_pack_layout(&gap).unwrap_err(),
            GitPackLayoutError::NonContiguous {
                previous_last_sequence: 4,
                next_first_sequence: 6,
            }
        );

        let overlap = [pack_span(1, 4, 2), pack_span(4, 4, 0)];
        assert_eq!(
            validate_git_pack_layout(&overlap).unwrap_err(),
            GitPackLayoutError::NonContiguous {
                previous_last_sequence: 4,
                next_first_sequence: 4,
            }
        );
    }

    #[test]
    fn git_pack_layout_rejects_a_tier_that_does_not_match_coverage() {
        let spans = [pack_span(1, 8, 2)];

        assert_eq!(
            validate_git_pack_layout(&spans).unwrap_err(),
            GitPackLayoutError::InvalidGeometricTier {
                first_sequence: 1,
                last_sequence: 8,
                actual: 2,
                expected: 3,
            }
        );
    }

    #[test]
    fn git_pack_layout_requires_power_of_two_coverage_in_descending_tier_order() {
        let uneven = [pack_span(1, 6, 2)];
        assert_eq!(
            validate_git_pack_layout(&uneven).unwrap_err(),
            GitPackLayoutError::NonGeometricCoverage {
                first_sequence: 1,
                last_sequence: 6,
                sequence_count: 6,
            }
        );

        let increasing = [pack_span(1, 1, 0), pack_span(2, 3, 1)];
        assert_eq!(
            validate_git_pack_layout(&increasing).unwrap_err(),
            GitPackLayoutError::IncreasingTier {
                previous_tier: 0,
                next_tier: 1,
            }
        );
    }

    #[test]
    fn git_pack_layout_requires_a_connected_history_boundary() {
        let mut first = pack_span(1, 2, 1);
        first.base_oid = Some("unexpected".to_string());
        assert!(matches!(
            validate_git_pack_layout(&[first]).unwrap_err(),
            GitPackLayoutError::InvalidStartBase { .. }
        ));

        let mut disconnected = [pack_span(1, 2, 1), pack_span(3, 3, 0)];
        disconnected[1].base_oid = Some("different".to_string());
        assert!(matches!(
            validate_git_pack_layout(&disconnected).unwrap_err(),
            GitPackLayoutError::DisconnectedHistory { .. }
        ));
    }
}
