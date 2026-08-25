mod projections;
mod repo_collaboration;
mod requests;

pub(crate) use projections::*;
pub(crate) use repo_collaboration::*;
pub(crate) use requests::*;
#[cfg(feature = "type-export")]
pub(crate) use scope_api_contract::CommitFileResponse;
use scope_api_contract::{
    DeviceLoginStatus, FileChangeKind, FirstPushTokenResponse, GitOid, GitPushTokenResponse,
    RepoInitResponse, RepoLifecycleState, RepoRequestPermissionsResponse, RepoSummaryResponse,
    RepositoryAccessResponse, RepositoryRunSummaryResponse, RequestActorSummaryResponse,
    SessionIdentity, UserResponse, Visibility,
};

use crate::{config::DEFAULT_GIT_BRANCH, error::ApiError};
use scope_domain::history::{
    HistoryEntry, HistoryEntryFile, HistoryEntryKind as DomainHistoryEntryKind,
    HistoryEntryVisibilityChange, HistoryView,
};
use scope_domain::policy::ScopePath;
use scope_domain::{
    account::UserAccount,
    repository::access::{RepositoryAccess, RepositoryActor},
    repository::credentials::{FirstPushToken, GitPushToken},
    repository::{RepoLifecycleState as DomainRepoLifecycleState, Repository},
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

pub(crate) fn request_actor_summary_response(
    user_id: &str,
    users: &BTreeMap<String, UserAccount>,
) -> Result<RequestActorSummaryResponse, ApiError> {
    let user = users
        .get(user_id)
        .ok_or_else(|| ApiError::internal_message("request actor was not persisted"))?;
    Ok(RequestActorSummaryResponse {
        id: user.id.clone(),
        handle: user.handle.clone(),
    })
}

#[derive(Debug, Serialize)]
#[cfg_attr(feature = "type-export", derive(ts_rs::TS))]
pub(crate) struct RepositoryRunWorkflowResponse {
    pub(crate) key: String,
    pub(crate) name: String,
    pub(crate) path: String,
    pub(crate) manual: bool,
    pub(crate) push_main: bool,
    pub(crate) job_count: usize,
}

#[derive(Debug, Serialize)]
#[cfg_attr(feature = "type-export", derive(ts_rs::TS))]
pub(crate) struct RepositoryRunWorkflowListResponse {
    pub(crate) workflows: Vec<RepositoryRunWorkflowResponse>,
}

#[derive(Debug, Serialize)]
#[cfg_attr(feature = "type-export", derive(ts_rs::TS))]
pub(crate) struct RepositoryRunHistoryPageResponse {
    pub(crate) runs: Vec<RepositoryRunSummaryResponse>,
    pub(crate) next_cursor: Option<String>,
}

#[derive(Debug, Serialize)]
#[cfg_attr(feature = "type-export", derive(ts_rs::TS))]
pub(crate) struct RepositoryRunLogResponse {
    pub(crate) position: u64,
    pub(crate) sequence: u64,
    pub(crate) text: String,
    pub(crate) created_at_unix: u64,
}

#[derive(Debug, Serialize)]
#[cfg_attr(feature = "type-export", derive(ts_rs::TS))]
pub(crate) struct RepositoryRunStepLogPageResponse {
    pub(crate) logs: Vec<RepositoryRunLogResponse>,
    pub(crate) next_after: u64,
    pub(crate) logs_truncated: bool,
}

#[derive(Debug, Serialize)]
pub(crate) struct HealthResponse {
    pub(crate) status: &'static str,
    pub(crate) service: &'static str,
}

#[derive(Debug, Serialize)]
pub(crate) struct ReadinessResponse {
    pub(crate) status: &'static str,
    pub(crate) service: &'static str,
    pub(crate) checks: Vec<ReadinessCheckResponse>,
}

#[derive(Debug, Serialize)]
pub(crate) struct ReadinessCheckResponse {
    pub(crate) name: &'static str,
    pub(crate) status: &'static str,
}

pub(crate) fn user_response(user: UserAccount) -> UserResponse {
    UserResponse {
        id: user.id,
        handle: user.handle,
        email: user.email,
        email_verified: user.email_verified,
    }
}

pub(crate) fn git_oid_response(value: String) -> Result<GitOid, ApiError> {
    GitOid::try_from(value)
        .map_err(|error| ApiError::internal_message(format!("persisted {error}")))
}

pub(crate) fn git_oid_request(label: &str, value: &str) -> Result<String, ApiError> {
    GitOid::try_from(value.trim())
        .map(String::from)
        .map_err(|_| ApiError::bad_request(format!("{label} must be a full SHA-1 Git object id")))
}

#[derive(Debug, Serialize)]
#[cfg_attr(feature = "type-export", derive(ts_rs::TS))]
pub(crate) struct SessionResponse {
    pub(crate) identity: Option<SessionIdentity>,
    pub(crate) repo: SessionRepo,
    pub(crate) principal_id: String,
    pub(crate) capabilities: SessionCapabilities,
}

#[derive(Debug, Serialize)]
#[cfg_attr(feature = "type-export", derive(ts_rs::TS))]
pub(crate) struct SessionRepo {
    pub(crate) id: String,
    pub(crate) lifecycle_state: RepoLifecycleState,
    pub(crate) access: RepositoryAccessResponse,
}

#[derive(Debug, Serialize)]
#[cfg_attr(feature = "type-export", derive(ts_rs::TS))]
pub(crate) struct SessionCapabilities {
    pub(crate) read: bool,
    pub(crate) can_read_private_files: bool,
    pub(crate) can_push: bool,
    pub(crate) can_change_file_visibility: bool,
    pub(crate) can_apply_changes: bool,
    pub(crate) can_manage_members: bool,
    pub(crate) can_delete_repo: bool,
}

#[derive(Debug, Serialize)]
#[cfg_attr(feature = "type-export", derive(ts_rs::TS))]
pub(crate) struct DeviceLoginCompleteResponse {
    pub(crate) status: DeviceLoginStatus,
}

#[derive(Debug, Serialize)]
#[cfg_attr(feature = "type-export", derive(ts_rs::TS))]
pub(crate) struct BrowserLoginCompleteResponse {
    pub(crate) callback_url: String,
}

#[derive(Debug, Serialize)]
#[cfg_attr(feature = "type-export", derive(ts_rs::TS))]
pub(crate) struct CliExchangeGrantResponse {
    pub(crate) exchange_token: String,
    pub(crate) expires_at_unix: u64,
}

#[derive(Debug, Serialize)]
#[cfg_attr(feature = "type-export", derive(ts_rs::TS))]
pub(crate) struct CliSessionsResponse {
    pub(crate) sessions: Vec<CliSessionResponse>,
}

#[derive(Debug, Serialize)]
#[cfg_attr(feature = "type-export", derive(ts_rs::TS))]
pub(crate) struct CliSessionResponse {
    pub(crate) id: String,
    pub(crate) label: String,
    pub(crate) created_at_unix: u64,
    pub(crate) last_used_at_unix: Option<u64>,
    pub(crate) expires_at_unix: u64,
}

#[derive(Debug, Serialize)]
#[cfg_attr(feature = "type-export", derive(ts_rs::TS))]
pub(crate) struct DeleteRepoResponse {
    pub(crate) id: String,
    pub(crate) deleted: bool,
}

#[derive(Debug, Deserialize)]
#[cfg_attr(feature = "type-export", derive(ts_rs::TS))]
pub(crate) struct HistoryPageRequest {
    pub(crate) audience: Option<ProjectionPreviewAudience>,
    pub(crate) before: Option<String>,
}

#[derive(Debug, Deserialize)]
#[cfg_attr(feature = "type-export", derive(ts_rs::TS))]
pub(crate) struct HistoryEntryRequest {
    pub(crate) audience: Option<ProjectionPreviewAudience>,
}

#[derive(Debug, Deserialize)]
#[cfg_attr(feature = "type-export", derive(ts_rs::TS))]
pub(crate) struct HistoryEntryFileDiffRequest {
    pub(crate) audience: Option<ProjectionPreviewAudience>,
    pub(crate) path: String,
}

#[derive(Debug, Deserialize)]
#[cfg_attr(feature = "type-export", derive(ts_rs::TS))]
pub(crate) struct RequestFileDiffRequest {
    pub(crate) path: String,
}

#[derive(Debug, Deserialize)]
#[cfg_attr(feature = "type-export", derive(ts_rs::TS))]
pub(crate) struct RepoFileContentRequest {
    pub(crate) path: String,
}

#[derive(Debug, Serialize)]
#[cfg_attr(feature = "type-export", derive(ts_rs::TS))]
pub(crate) struct ReviewFileDiffResponse {
    pub(crate) path: String,
    pub(crate) kind: FileChangeKind,
    pub(crate) old_mode: Option<String>,
    pub(crate) new_mode: Option<String>,
    pub(crate) old_content: Option<ReviewFileContentResponse>,
    pub(crate) new_content: Option<ReviewFileContentResponse>,
}

#[derive(Debug, Serialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
#[cfg_attr(feature = "type-export", derive(ts_rs::TS))]
#[cfg_attr(feature = "type-export", ts(rename_all = "lowercase"))]
pub(crate) enum ReviewFileContentResponse {
    Text { text: String },
    Binary { oid: String, size_bytes: u64 },
}

#[derive(Debug, Serialize)]
#[cfg_attr(feature = "type-export", derive(ts_rs::TS))]
pub(crate) struct HistoryPageResponse {
    pub(crate) audience: ProjectionPreviewAudience,
    pub(crate) repo_id: String,
    pub(crate) view_key: String,
    pub(crate) generation: String,
    pub(crate) entries: Vec<HistoryEntrySummaryResponse>,
    pub(crate) next_cursor: Option<String>,
}

#[derive(Debug, Serialize)]
#[cfg_attr(feature = "type-export", derive(ts_rs::TS))]
pub(crate) struct HistoryEntrySummaryResponse {
    pub(crate) id: String,
    pub(crate) source_id: String,
    pub(crate) parent_id: Option<String>,
    pub(crate) kind: HistoryEntryKind,
    pub(crate) author: Option<String>,
    pub(crate) message: String,
    pub(crate) file_change_count: usize,
    pub(crate) visibility_summary: HistoryVisibilitySummaryResponse,
}

#[derive(Clone, Copy, Debug, Serialize)]
#[serde(rename_all = "snake_case")]
#[cfg_attr(feature = "type-export", derive(ts_rs::TS))]
#[cfg_attr(feature = "type-export", ts(rename_all = "snake_case"))]
pub(crate) enum HistoryEntryKind {
    Push,
    MergedRequest,
    VisibilityChange,
}

#[derive(Debug, Serialize)]
#[cfg_attr(feature = "type-export", derive(ts_rs::TS))]
pub(crate) struct HistoryEntryDetailResponse {
    pub(crate) audience: ProjectionPreviewAudience,
    pub(crate) repo_id: String,
    pub(crate) view_key: String,
    pub(crate) id: String,
    pub(crate) source_id: String,
    pub(crate) parent_id: Option<String>,
    pub(crate) kind: HistoryEntryKind,
    pub(crate) author: Option<String>,
    pub(crate) message: String,
    pub(crate) file_change_count: usize,
    pub(crate) visibility_summary: HistoryVisibilitySummaryResponse,
    pub(crate) files: Vec<HistoryEntryFileResponse>,
    pub(crate) visibility_changes: Vec<HistoryVisibilityChangeResponse>,
}

#[derive(Debug, Serialize)]
#[cfg_attr(feature = "type-export", derive(ts_rs::TS))]
pub(crate) struct HistoryVisibilitySummaryResponse {
    pub(crate) made_public_count: usize,
    pub(crate) made_private_count: usize,
}

#[derive(Debug, Serialize)]
#[cfg_attr(feature = "type-export", derive(ts_rs::TS))]
pub(crate) struct HistoryVisibilityChangeResponse {
    pub(crate) path: String,
    pub(crate) old_visibility: Visibility,
    pub(crate) new_visibility: Visibility,
}

#[derive(Debug, Serialize)]
#[cfg_attr(feature = "type-export", derive(ts_rs::TS))]
pub(crate) struct HistoryEntryFileResponse {
    pub(crate) path: String,
    pub(crate) kind: FileChangeKind,
    pub(crate) old_mode: Option<String>,
    pub(crate) new_mode: Option<String>,
    pub(crate) old_oid: Option<String>,
    pub(crate) new_oid: Option<String>,
    pub(crate) visibility: Visibility,
}

pub(crate) fn repo_summary_for_user(
    repo: &Repository,
    user_id: &str,
    open_request_count: usize,
) -> Option<RepoSummaryResponse> {
    let access = repo.access_for_user_id(user_id);
    if access.actor == RepositoryActor::Public {
        return None;
    }
    let lifecycle_allows_read = repo.record.lifecycle_state == DomainRepoLifecycleState::Ready
        || access.actor == RepositoryActor::Owner;
    if !lifecycle_allows_read
        || !repo
            .policy
            .can_read(&ScopePath::root(), access.can_read_private_files)
    {
        return None;
    }

    Some(RepoSummaryResponse {
        id: repo.record.id.clone(),
        owner_handle: repo.record.owner_handle.clone(),
        name: repo.record.name.clone(),
        lifecycle_state: repo.record.lifecycle_state.into(),
        change_version: repo_change_version_for_access(repo, access),
        access: repository_access_response(access),
        open_request_count,
        request_permissions: repo_request_permissions_response(access),
    })
}

pub(crate) fn repo_request_permissions_response(
    _access: RepositoryAccess,
) -> RepoRequestPermissionsResponse {
    RepoRequestPermissionsResponse {
        can_start_request: true,
    }
}

pub(crate) fn repo_change_version_for_access(repo: &Repository, access: RepositoryAccess) -> u64 {
    if access.actor != RepositoryActor::Public {
        repo.record.change_version
    } else {
        0
    }
}

pub(crate) fn repository_access_response(access: RepositoryAccess) -> RepositoryAccessResponse {
    RepositoryAccessResponse {
        actor: access.actor.into(),
        can_read_private_files: access.can_read_private_files,
        can_push: access.can_push,
        can_change_file_visibility: access.can_change_file_visibility,
        can_apply_changes: access.can_apply_changes,
        can_manage_members: access.can_manage_members,
        can_delete_repo: access.can_delete_repo,
    }
}

pub(crate) fn session_capabilities_response(
    read: bool,
    access: RepositoryAccess,
) -> SessionCapabilities {
    SessionCapabilities {
        read,
        can_read_private_files: access.can_read_private_files,
        can_push: access.can_push,
        can_change_file_visibility: access.can_change_file_visibility,
        can_apply_changes: access.can_apply_changes,
        can_manage_members: access.can_manage_members,
        can_delete_repo: access.can_delete_repo,
    }
}

pub(crate) fn repo_init_response(
    repo: &Repository,
    user_id: &str,
    api_origin: &str,
    now_unix: u64,
    secret: Option<String>,
    push_secret: Option<String>,
) -> Result<RepoInitResponse, ApiError> {
    ensure_repo_init_access(repo, user_id)?;
    let repo_summary = repo_summary_for_user(repo, user_id, 0)
        .ok_or_else(|| ApiError::internal_message("init repository is not readable"))?;
    let token = repo
        .first_push_token
        .as_ref()
        .map(|stored_token| first_push_token_response(stored_token, now_unix, secret));
    let push_token = repo
        .git_push_token
        .as_ref()
        .map(|stored_token| git_push_token_response(stored_token, push_secret));

    let git_remote_path = scope_api_contract::routes::git_repo(
        "permissioned",
        &repo_summary.owner_handle,
        &repo_summary.name,
    );
    Ok(RepoInitResponse {
        git_remote_url: format!("{}{}", api_origin.trim_end_matches('/'), git_remote_path),
        remote_name: "scope".to_string(),
        push_branch: DEFAULT_GIT_BRANCH.to_string(),
        repo: repo_summary,
        token,
        push_token,
    })
}

fn ensure_repo_init_access(repo: &Repository, user_id: &str) -> Result<(), ApiError> {
    if !repo.is_owner_user(user_id) {
        return Err(ApiError::not_found(format!(
            "repo {} not found",
            repo.record.id
        )));
    }
    if !repo.is_waiting_for_first_push() {
        return Err(ApiError::conflict(
            "init token is only available before the first push",
        ));
    }
    Ok(())
}

pub(crate) fn first_push_token_response(
    token: &FirstPushToken,
    now_unix: u64,
    secret: Option<String>,
) -> FirstPushTokenResponse {
    let status = token.status_at(now_unix);
    let secret = if status == scope_domain::repository::credentials::FirstPushTokenStatus::Active {
        secret
    } else {
        None
    };

    FirstPushTokenResponse {
        status: status.into(),
        created_at_unix: token.created_at_unix,
        expires_at_unix: token.expires_at_unix,
        used_at_unix: token.used_at_unix,
        secret,
    }
}

pub(crate) fn git_push_token_response(
    token: &GitPushToken,
    secret: Option<String>,
) -> GitPushTokenResponse {
    GitPushTokenResponse {
        created_at_unix: token.created_at_unix,
        secret,
    }
}

pub(crate) fn history_page_response(
    audience: ProjectionPreviewAudience,
    view: &HistoryView,
    entries: &[HistoryEntry],
    next_cursor: Option<String>,
) -> HistoryPageResponse {
    HistoryPageResponse {
        audience,
        repo_id: view.repo_id.clone(),
        view_key: view.view_key.clone(),
        generation: view.generation.clone(),
        entries: entries.iter().map(history_entry_summary_response).collect(),
        next_cursor,
    }
}

pub(crate) fn history_entry_detail_response(
    audience: ProjectionPreviewAudience,
    view: &HistoryView,
    entry: &HistoryEntry,
) -> HistoryEntryDetailResponse {
    HistoryEntryDetailResponse {
        audience,
        repo_id: view.repo_id.clone(),
        view_key: view.view_key.clone(),
        id: entry.id.clone(),
        source_id: entry.source_id.clone(),
        parent_id: entry.parent_id.clone(),
        kind: entry.kind.into(),
        author: entry.author.clone(),
        message: entry.message.clone(),
        file_change_count: entry.files.len(),
        visibility_summary: history_visibility_summary_response(entry),
        files: entry
            .files
            .iter()
            .map(history_entry_file_response)
            .collect(),
        visibility_changes: entry
            .visibility_changes
            .iter()
            .map(history_visibility_change_response)
            .collect(),
    }
}

fn history_entry_summary_response(entry: &HistoryEntry) -> HistoryEntrySummaryResponse {
    HistoryEntrySummaryResponse {
        id: entry.id.clone(),
        source_id: entry.source_id.clone(),
        parent_id: entry.parent_id.clone(),
        kind: entry.kind.into(),
        author: entry.author.clone(),
        message: entry.message.clone(),
        file_change_count: entry.files.len(),
        visibility_summary: history_visibility_summary_response(entry),
    }
}

fn history_visibility_summary_response(entry: &HistoryEntry) -> HistoryVisibilitySummaryResponse {
    let made_public_count = entry
        .visibility_changes
        .iter()
        .filter(|change| change.new_visibility == scope_domain::policy::Visibility::Public)
        .count();
    HistoryVisibilitySummaryResponse {
        made_public_count,
        made_private_count: entry.visibility_changes.len() - made_public_count,
    }
}

fn history_visibility_change_response(
    change: &HistoryEntryVisibilityChange,
) -> HistoryVisibilityChangeResponse {
    HistoryVisibilityChangeResponse {
        path: change.path.as_str().to_string(),
        old_visibility: change.old_visibility.into(),
        new_visibility: change.new_visibility.into(),
    }
}

impl From<DomainHistoryEntryKind> for HistoryEntryKind {
    fn from(kind: DomainHistoryEntryKind) -> Self {
        match kind {
            DomainHistoryEntryKind::Push => Self::Push,
            DomainHistoryEntryKind::MergedRequest => Self::MergedRequest,
            DomainHistoryEntryKind::VisibilityChange => Self::VisibilityChange,
        }
    }
}

fn history_entry_file_response(file: &HistoryEntryFile) -> HistoryEntryFileResponse {
    HistoryEntryFileResponse {
        path: file.path.as_str().to_string(),
        kind: file.kind.into(),
        old_mode: file
            .old_content
            .as_ref()
            .map(|blob| blob.git_file_mode.clone()),
        new_mode: file
            .new_content
            .as_ref()
            .map(|blob| blob.git_file_mode.clone()),
        old_oid: file.old_content.as_ref().map(|blob| blob.git_oid.clone()),
        new_oid: file.new_content.as_ref().map(|blob| blob.git_oid.clone()),
        visibility: file.visibility.into(),
    }
}
