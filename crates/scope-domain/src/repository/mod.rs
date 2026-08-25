pub mod access;
pub mod collaboration;
pub mod credentials;
pub mod git;
pub mod updates;

use crate::{
    account::UserAccount,
    content::SourceBlob,
    policy::{Policy, PolicyError, ScopePath, Visibility},
    projection::SourceGraph,
    repo_config::{ConfigVisibility, RepoConfig},
    repository::{
        collaboration::{RepositoryInvite, RepositoryMember},
        credentials::{FirstPushToken, GitPushToken},
        git::{GitHead, GitPackSpan},
    },
    visibility_changes::VisibilityChangeSet,
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum RepoLifecycleState {
    AwaitingFirstPush,
    Ready,
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

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Repository {
    pub record: RepoRecord,
    pub repo_config: RepoConfig,
    pub first_push_token: Option<FirstPushToken>,
    pub git_push_token: Option<GitPushToken>,
    pub policy: Policy,
    pub graph: SourceGraph,
    pub visibility_change_sets: Vec<VisibilityChangeSet>,
    pub live_files: BTreeMap<ScopePath, SourceBlob>,
    pub git_head: Option<GitHead>,
    pub git_pack_spans: Vec<GitPackSpan>,
    pub members: Vec<RepositoryMember>,
    pub invitations: Vec<RepositoryInvite>,
}

impl Repository {
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
            visibility_change_sets: Vec::new(),
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
        for change in self
            .visibility_change_sets
            .iter()
            .flat_map(|set| &set.changes)
        {
            blobs.extend(change.current_content.clone());
        }
        blobs
    }

    pub fn has_file_for_visibility_update(&self, path: &ScopePath) -> bool {
        self.graph_has_file(path)
    }
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
