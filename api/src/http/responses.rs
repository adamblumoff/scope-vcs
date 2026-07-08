mod projections;
mod repo_collaboration;
mod requests;

pub(crate) use projections::*;
pub(crate) use repo_collaboration::*;
pub(crate) use requests::*;

use crate::domain::commit_history::{CommitHistoryCommit, CommitHistoryView};
use crate::domain::policy::{ScopePath, Visibility};
use crate::domain::repo_config::RepoConfig;
use crate::domain::store::{
    FirstPushToken, FirstPushTokenStatus, GitPushToken, RepoPublicationState, RepositoryAccess,
    RepositoryActor, StagedFileChangeKind, StoredRepository, UserAccount,
};
use crate::{config::DEFAULT_GIT_BRANCH, error::ApiError};
pub(crate) use scope_core::auth::device::SessionIdentity;
use serde::{Deserialize, Serialize};

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

#[derive(Debug, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub(crate) struct AccountSessionResponse {
    pub(crate) identity: Option<SessionIdentity>,
    pub(crate) user: Option<UserResponse>,
}

#[derive(Debug, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub(crate) struct UserResponse {
    pub(crate) id: String,
    pub(crate) handle: String,
    pub(crate) email: String,
    pub(crate) email_verified: bool,
}

impl From<UserAccount> for UserResponse {
    fn from(user: UserAccount) -> Self {
        Self {
            id: user.id,
            handle: user.handle,
            email: user.email,
            email_verified: user.email_verified,
        }
    }
}

#[derive(Debug, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub(crate) struct SessionResponse {
    pub(crate) identity: Option<SessionIdentity>,
    pub(crate) repo: SessionRepo,
    pub(crate) principal_id: String,
    pub(crate) capabilities: SessionCapabilities,
}

#[derive(Debug, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub(crate) struct SessionRepo {
    pub(crate) id: String,
    pub(crate) publication_state: RepoPublicationState,
    pub(crate) access: RepositoryAccessResponse,
}

#[derive(Debug, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub(crate) struct SessionCapabilities {
    pub(crate) read: bool,
    pub(crate) can_read_private_files: bool,
    pub(crate) can_push: bool,
    pub(crate) can_change_file_visibility: bool,
    pub(crate) can_apply_changes: bool,
    pub(crate) can_update_repo_settings: bool,
    pub(crate) can_manage_members: bool,
    pub(crate) can_delete_repo: bool,
}

#[derive(Clone, Copy, Debug, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub(crate) enum DeviceLoginStatus {
    Pending,
    Complete,
}

#[derive(Debug, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub(crate) struct DeviceLoginStartResponse {
    pub(crate) device_code: String,
    pub(crate) user_code: String,
    pub(crate) verification_url: String,
    pub(crate) expires_at_unix: u64,
    pub(crate) poll_interval_secs: u64,
}

#[derive(Debug, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub(crate) struct DeviceLoginPollResponse {
    pub(crate) status: DeviceLoginStatus,
    pub(crate) session_token: Option<String>,
    pub(crate) expires_at_unix: u64,
    pub(crate) identity: Option<SessionIdentity>,
}

#[derive(Debug, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub(crate) struct DeviceLoginCompleteResponse {
    pub(crate) status: DeviceLoginStatus,
}

#[derive(Debug, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub(crate) struct BrowserLoginStartRequest {
    pub(crate) callback_url: String,
}

#[derive(Debug, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub(crate) struct BrowserLoginStartResponse {
    pub(crate) request_id: String,
    pub(crate) request_secret: String,
    pub(crate) authorization_url: String,
    pub(crate) expires_at_unix: u64,
}

#[derive(Debug, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub(crate) struct BrowserLoginCompleteResponse {
    pub(crate) callback_url: String,
}

#[derive(Debug, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub(crate) struct BrowserLoginExchangeRequest {
    pub(crate) request_secret: String,
    pub(crate) callback_code: String,
}

#[derive(Debug, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub(crate) struct CliSessionTokenResponse {
    pub(crate) session_token: String,
    pub(crate) expires_at_unix: u64,
    pub(crate) identity: SessionIdentity,
}

#[derive(Debug, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub(crate) struct CliExchangeGrantResponse {
    pub(crate) exchange_token: String,
    pub(crate) expires_at_unix: u64,
}

#[derive(Debug, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub(crate) struct CliExchangeGrantExchangeRequest {
    pub(crate) exchange_token: String,
}

#[derive(Debug, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub(crate) struct CliSessionsResponse {
    pub(crate) sessions: Vec<CliSessionResponse>,
}

#[derive(Debug, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub(crate) struct CliSessionResponse {
    pub(crate) id: String,
    pub(crate) label: String,
    pub(crate) created_at_unix: u64,
    pub(crate) last_used_at_unix: Option<u64>,
    pub(crate) expires_at_unix: u64,
}

#[derive(Debug, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub(crate) struct RepoSummaryResponse {
    pub(crate) id: String,
    pub(crate) owner_handle: String,
    pub(crate) name: String,
    pub(crate) lifecycle_state: RepoPublicationState,
    pub(crate) default_visibility: Visibility,
    pub(crate) change_version: u64,
    pub(crate) access: RepositoryAccessResponse,
    pub(crate) pending_import_pending: bool,
    pub(crate) open_request_count: usize,
    pub(crate) request_permissions: RepoRequestPermissionsResponse,
}

#[derive(Debug, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub(crate) struct RepositoryAccessResponse {
    pub(crate) actor: RepositoryActor,
    pub(crate) can_read_private_files: bool,
    pub(crate) can_push: bool,
    pub(crate) can_change_file_visibility: bool,
    pub(crate) can_apply_changes: bool,
    pub(crate) can_update_repo_settings: bool,
    pub(crate) can_manage_members: bool,
    pub(crate) can_delete_repo: bool,
}

#[derive(Debug, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub(crate) struct CreateRepoResponse {
    pub(crate) repo: RepoSummaryResponse,
    pub(crate) init: RepoInitResponse,
}

#[derive(Debug, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub(crate) struct DeleteRepoResponse {
    pub(crate) id: String,
    pub(crate) deleted: bool,
}

#[derive(Debug, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub(crate) struct CreatePushIntentRequest {
    pub(crate) head_oid: String,
    pub(crate) base_config_hash: String,
    pub(crate) config: RepoConfig,
}

#[derive(Debug, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub(crate) struct CreatePushIntentResponse {
    pub(crate) token: String,
    pub(crate) base_head_oid: Option<String>,
    pub(crate) expires_at_unix: u64,
}

#[derive(Debug, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub(crate) struct CompletePushIntentRequest {
    pub(crate) token: String,
}

#[derive(Debug, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub(crate) struct CompletePushIntentResponse {
    pub(crate) config_applied: bool,
}

#[derive(Debug, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub(crate) struct RepoInitResponse {
    pub(crate) repo: RepoSummaryResponse,
    pub(crate) git_remote_url: String,
    pub(crate) remote_name: &'static str,
    pub(crate) push_branch: &'static str,
    pub(crate) token: Option<FirstPushTokenResponse>,
    pub(crate) push_token: Option<GitPushTokenResponse>,
}

#[derive(Debug, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub(crate) struct RepoConfigResponse {
    pub(crate) config: RepoConfig,
    pub(crate) config_hash: String,
}

#[derive(Debug, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub(crate) struct FirstPushTokenResponse {
    pub(crate) status: FirstPushTokenStatus,
    pub(crate) created_at_unix: u64,
    pub(crate) expires_at_unix: u64,
    pub(crate) used_at_unix: Option<u64>,
    pub(crate) secret: Option<String>,
}

#[derive(Debug, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub(crate) struct GitPushTokenResponse {
    pub(crate) created_at_unix: u64,
    pub(crate) secret: Option<String>,
}

#[derive(Debug, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub(crate) struct CommitHistoryRequest {
    pub(crate) audience: Option<ProjectionPreviewAudience>,
}

#[derive(Debug, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub(crate) struct CommitFileDiffRequest {
    pub(crate) audience: Option<ProjectionPreviewAudience>,
    pub(crate) path: String,
}

#[derive(Debug, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub(crate) struct ReviewFileDiffResponse {
    pub(crate) path: String,
    pub(crate) kind: StagedFileChangeKind,
    pub(crate) old_content: Option<ReviewFileContentResponse>,
    pub(crate) new_content: Option<ReviewFileContentResponse>,
}

#[derive(Debug, Serialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
#[cfg_attr(test, derive(ts_rs::TS))]
#[cfg_attr(test, ts(rename_all = "lowercase"))]
pub(crate) enum ReviewFileContentResponse {
    Text { text: String },
    Binary { oid: String, size_bytes: u64 },
}

#[derive(Debug, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub(crate) struct CommitHistoryResponse {
    pub(crate) audience: ProjectionPreviewAudience,
    pub(crate) repo_id: String,
    pub(crate) view_key: String,
    pub(crate) commits: Vec<CommitSummaryResponse>,
}

#[derive(Debug, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub(crate) struct CommitSummaryResponse {
    pub(crate) projected_id: String,
    pub(crate) logical_commit_id: String,
    pub(crate) parent_projected_id: Option<String>,
    pub(crate) author: Option<String>,
    pub(crate) message: String,
    pub(crate) change_count: usize,
}

#[derive(Debug, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub(crate) struct CommitDetailResponse {
    pub(crate) audience: ProjectionPreviewAudience,
    pub(crate) repo_id: String,
    pub(crate) view_key: String,
    pub(crate) projected_id: String,
    pub(crate) logical_commit_id: String,
    pub(crate) parent_projected_id: Option<String>,
    pub(crate) author: Option<String>,
    pub(crate) message: String,
    pub(crate) change_count: usize,
    pub(crate) files: Vec<CommitFileResponse>,
}

#[derive(Debug, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub(crate) struct CommitFileResponse {
    pub(crate) path: String,
    pub(crate) kind: StagedFileChangeKind,
    pub(crate) old_oid: Option<String>,
    pub(crate) new_oid: Option<String>,
    pub(crate) visibility: Visibility,
}

#[derive(Debug, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub(crate) struct CreateRepoRequest {
    pub(crate) name: String,
    pub(crate) visibility: Option<Visibility>,
}

pub(crate) fn repo_summary_for_user(
    repo: &StoredRepository,
    user_id: &str,
    open_request_count: usize,
) -> Option<RepoSummaryResponse> {
    let access = repo.access_for_user_id(user_id);
    if access.actor == RepositoryActor::Public {
        return None;
    }
    let lifecycle_allows_read = repo.record.publication_state == RepoPublicationState::Published
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
        lifecycle_state: repo.record.publication_state,
        default_visibility: repo.record.default_visibility,
        change_version: repo_change_version_for_access(repo, access),
        access: repository_access_response(access),
        pending_import_pending: repo.has_pending_import_review(),
        open_request_count,
        request_permissions: repo_request_permissions_response(access),
    })
}

pub(crate) fn repo_request_permissions_response(
    access: RepositoryAccess,
) -> RepoRequestPermissionsResponse {
    RepoRequestPermissionsResponse {
        can_submit_request: true,
        uses_credit_stake: access.actor == RepositoryActor::Public,
    }
}

pub(crate) fn repo_change_version_for_access(
    repo: &StoredRepository,
    access: RepositoryAccess,
) -> u64 {
    if access.actor != RepositoryActor::Public {
        repo.record.change_version
    } else {
        0
    }
}

pub(crate) fn repository_access_response(access: RepositoryAccess) -> RepositoryAccessResponse {
    RepositoryAccessResponse {
        actor: access.actor,
        can_read_private_files: access.can_read_private_files,
        can_push: access.can_push,
        can_change_file_visibility: access.can_change_file_visibility,
        can_apply_changes: access.can_apply_changes,
        can_update_repo_settings: access.can_update_repo_settings,
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
        can_update_repo_settings: access.can_update_repo_settings,
        can_manage_members: access.can_manage_members,
        can_delete_repo: access.can_delete_repo,
    }
}

pub(crate) fn repo_init_response(
    repo: &StoredRepository,
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

    let git_remote_path = format!(
        "/git/permissioned/{}/{}",
        repo_summary.owner_handle, repo_summary.name
    );
    Ok(RepoInitResponse {
        git_remote_url: format!("{}{}", api_origin.trim_end_matches('/'), git_remote_path),
        remote_name: "scope",
        push_branch: DEFAULT_GIT_BRANCH,
        repo: repo_summary,
        token,
        push_token,
    })
}

fn ensure_repo_init_access(repo: &StoredRepository, user_id: &str) -> Result<(), ApiError> {
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
    let secret = if status == FirstPushTokenStatus::Active {
        secret
    } else {
        None
    };

    FirstPushTokenResponse {
        status,
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

pub(crate) fn commit_history_response(
    audience: ProjectionPreviewAudience,
    view: CommitHistoryView,
) -> CommitHistoryResponse {
    CommitHistoryResponse {
        audience,
        repo_id: view.repo_id,
        view_key: view.view_key,
        commits: view.commits.iter().map(commit_summary_response).collect(),
    }
}

pub(crate) fn commit_detail_response(
    audience: ProjectionPreviewAudience,
    view: &CommitHistoryView,
    commit: &CommitHistoryCommit,
) -> CommitDetailResponse {
    CommitDetailResponse {
        audience,
        repo_id: view.repo_id.clone(),
        view_key: view.view_key.clone(),
        projected_id: commit.projected_id.clone(),
        logical_commit_id: commit.logical_commit_id.clone(),
        parent_projected_id: commit.parent_projected_id.clone(),
        author: commit.author.clone(),
        message: commit.message.clone(),
        change_count: commit.files.len(),
        files: commit.files.iter().map(commit_file_response).collect(),
    }
}

fn commit_summary_response(commit: &CommitHistoryCommit) -> CommitSummaryResponse {
    CommitSummaryResponse {
        projected_id: commit.projected_id.clone(),
        logical_commit_id: commit.logical_commit_id.clone(),
        parent_projected_id: commit.parent_projected_id.clone(),
        author: commit.author.clone(),
        message: commit.message.clone(),
        change_count: commit.files.len(),
    }
}

fn commit_file_response(
    file: &crate::domain::commit_history::CommitHistoryFile,
) -> CommitFileResponse {
    CommitFileResponse {
        path: file.path.as_str().to_string(),
        kind: file.kind,
        old_oid: file.old_content.as_ref().map(|blob| blob.git_oid.clone()),
        new_oid: file.new_content.as_ref().map(|blob| blob.git_oid.clone()),
        visibility: file.visibility,
    }
}
