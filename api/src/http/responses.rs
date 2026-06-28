mod projections;

pub(crate) use projections::*;

use crate::domain::commit_history::{CommitHistoryCommit, CommitHistoryView};
use crate::domain::policy::{Principal, ScopePath, Visibility};
use crate::domain::store::{
    FirstPushToken, FirstPushTokenStatus, GitCloneToken, GitPushToken, RepoPublicationState,
    RepoRole, StagedFileChangeKind, StagedRepoUpdate, StoredRepository, UserAccount,
};
use crate::{config::DEFAULT_GIT_BRANCH, error::ApiError};
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
pub(crate) struct SessionIdentity {
    pub(crate) user_id: String,
    pub(crate) email: Option<String>,
    pub(crate) email_verified: bool,
}

impl From<&UserAccount> for SessionIdentity {
    fn from(user: &UserAccount) -> Self {
        Self {
            user_id: user.id.clone(),
            email: (!user.email.is_empty()).then(|| user.email.clone()),
            email_verified: user.email_verified,
        }
    }
}

#[derive(Debug, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub(crate) struct SessionRepo {
    pub(crate) id: String,
    pub(crate) publication_state: RepoPublicationState,
    pub(crate) role: Option<RepoRole>,
}

#[derive(Debug, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub(crate) struct SessionCapabilities {
    pub(crate) read: bool,
    pub(crate) write: bool,
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
    pub(crate) role: Option<RepoRole>,
    pub(crate) staged_update_pending: bool,
    pub(crate) push_blocked_by_staged_update: bool,
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

#[derive(Debug, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub(crate) struct RepoInitResponse {
    pub(crate) repo: RepoSummaryResponse,
    pub(crate) git_remote_url: String,
    pub(crate) remote_name: &'static str,
    pub(crate) push_branch: &'static str,
    pub(crate) token: Option<FirstPushTokenResponse>,
    pub(crate) push_token: Option<GitPushTokenResponse>,
    pub(crate) review_url: String,
}

#[derive(Debug, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub(crate) struct RepoCloneCredentialResponse {
    pub(crate) git_remote_path: String,
    pub(crate) token: GitCloneTokenResponse,
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

#[derive(Debug, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub(crate) struct GitCloneTokenResponse {
    pub(crate) created_at_unix: u64,
    pub(crate) secret: Option<String>,
}

#[derive(Debug, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub(crate) struct PendingImportReviewResponse {
    pub(crate) publication_state: RepoPublicationState,
    pub(crate) default_visibility: Visibility,
    pub(crate) line_diff: ReviewLineDiffResponse,
    pub(crate) files: Vec<RepoFileResponse>,
}

#[derive(Debug, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub(crate) struct UpdateFileVisibilityRequest {
    pub(crate) paths: Vec<String>,
    pub(crate) visibility: Visibility,
}

#[derive(Debug, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub(crate) struct RepoSettingsResponse {
    pub(crate) default_new_file_visibility: Visibility,
    pub(crate) review_pushes_before_applying: bool,
}

#[derive(Debug, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub(crate) struct UpdateRepoSettingsRequest {
    pub(crate) default_new_file_visibility: Visibility,
    pub(crate) review_pushes_before_applying: bool,
}

#[derive(Debug, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub(crate) struct UpdateStagedFileVisibilityRequest {
    pub(crate) paths: Vec<String>,
    pub(crate) visibility: Visibility,
}

#[derive(Debug, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub(crate) struct ReviewFileDiffRequest {
    pub(crate) path: String,
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
    pub(crate) old_content: Option<String>,
    pub(crate) new_content: Option<String>,
}

#[derive(Debug, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub(crate) struct ReviewLineDiffResponse {
    pub(crate) additions: usize,
    pub(crate) deletions: usize,
}

#[derive(Debug, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub(crate) struct StagedUpdateResponse {
    pub(crate) id: String,
    pub(crate) branch: String,
    pub(crate) base_live_commit_id: Option<String>,
    pub(crate) message: String,
    pub(crate) line_diff: ReviewLineDiffResponse,
    pub(crate) files: Vec<StagedFileResponse>,
}

#[derive(Debug, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub(crate) struct StagedFileResponse {
    pub(crate) path: String,
    pub(crate) kind: StagedFileChangeKind,
    pub(crate) old_oid: Option<String>,
    pub(crate) new_oid: Option<String>,
    pub(crate) visibility: Visibility,
}

#[derive(Debug, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub(crate) struct CommitHistoryResponse {
    pub(crate) audience: ProjectionPreviewAudience,
    pub(crate) repo_id: String,
    pub(crate) principal_id: String,
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
    pub(crate) principal_id: String,
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
) -> Option<RepoSummaryResponse> {
    let principal = Principal {
        id: user_id.to_string(),
        kind: crate::domain::policy::PrincipalKind::User,
    };
    let role = repo
        .memberships
        .iter()
        .find(|membership| membership.user_id == user_id)
        .map(|membership| membership.role)?;
    let lifecycle_allows_read =
        repo.record.publication_state == RepoPublicationState::Published || role == RepoRole::Owner;
    if !lifecycle_allows_read || !repo.policy.can_read(&principal, &ScopePath::root()) {
        return None;
    }

    Some(RepoSummaryResponse {
        id: repo.record.id.clone(),
        owner_handle: repo.record.owner_handle.clone(),
        name: repo.record.name.clone(),
        lifecycle_state: repo.record.publication_state,
        default_visibility: repo.record.default_visibility,
        change_version: repo.record.change_version,
        role: Some(role),
        staged_update_pending: role == RepoRole::Owner && repo.staged_update.is_some(),
        push_blocked_by_staged_update: role >= RepoRole::Writer && repo.staged_update.is_some(),
    })
}

pub(crate) fn repo_init_response(
    repo: &StoredRepository,
    user_id: &str,
    api_origin: &str,
    app_origin: &str,
    now_unix: u64,
    secret: Option<String>,
    push_secret: Option<String>,
) -> Result<RepoInitResponse, ApiError> {
    ensure_repo_init_access(repo, user_id)?;
    let repo_summary = repo_summary_for_user(repo, user_id)
        .ok_or_else(|| ApiError::internal_message("init repository is not readable"))?;
    let token = repo
        .first_push_token
        .as_ref()
        .map(|stored_token| first_push_token_response(stored_token, now_unix, secret));
    let push_token = repo
        .git_push_token
        .as_ref()
        .map(|stored_token| git_push_token_response(stored_token, push_secret));

    let git_remote_path = format!("/git/{}/{}", repo_summary.owner_handle, repo_summary.name);
    Ok(RepoInitResponse {
        git_remote_url: format!("{}{}", api_origin.trim_end_matches('/'), git_remote_path),
        remote_name: "scope",
        push_branch: DEFAULT_GIT_BRANCH,
        review_url: format!(
            "{}/repos/{}/{}/review",
            app_origin.trim_end_matches('/'),
            repo_summary.owner_handle,
            repo_summary.name
        ),
        repo: repo_summary,
        token,
        push_token,
    })
}

fn ensure_repo_init_access(repo: &StoredRepository, user_id: &str) -> Result<(), ApiError> {
    let role = repo
        .memberships
        .iter()
        .find(|membership| membership.user_id == user_id)
        .map(|membership| membership.role);
    if role != Some(RepoRole::Owner) {
        return Err(ApiError::not_found(format!(
            "repo {} not found",
            repo.record.id
        )));
    }
    if repo.record.publication_state != RepoPublicationState::PendingFirstPush {
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

pub(crate) fn git_clone_token_response(
    token: &GitCloneToken,
    secret: Option<String>,
) -> GitCloneTokenResponse {
    GitCloneTokenResponse {
        created_at_unix: token.created_at_unix,
        secret,
    }
}

pub(crate) fn repo_clone_credential_response(
    repo: &StoredRepository,
    token: &GitCloneToken,
    secret: Option<String>,
) -> RepoCloneCredentialResponse {
    RepoCloneCredentialResponse {
        git_remote_path: format!("/git/{}/{}", repo.record.owner_handle, repo.record.name),
        token: git_clone_token_response(token, secret),
    }
}

pub(crate) fn staged_update_response(
    update: &StagedRepoUpdate,
    line_diff: ReviewLineDiffResponse,
) -> StagedUpdateResponse {
    StagedUpdateResponse {
        id: update.id.clone(),
        branch: update.branch.clone(),
        base_live_commit_id: update.base_live_commit_id.clone(),
        message: update.message.clone(),
        line_diff,
        files: update
            .changes
            .iter()
            .map(|change| StagedFileResponse {
                path: change.path.as_str().to_string(),
                kind: change.kind,
                old_oid: change.old_content.as_ref().map(|blob| blob.git_oid.clone()),
                new_oid: change.new_content.as_ref().map(|blob| blob.git_oid.clone()),
                visibility: change.visibility,
            })
            .collect(),
    }
}

pub(crate) fn commit_history_response(
    audience: ProjectionPreviewAudience,
    view: CommitHistoryView,
) -> CommitHistoryResponse {
    CommitHistoryResponse {
        audience,
        repo_id: view.repo_id,
        principal_id: view.principal_id,
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
        principal_id: view.principal_id.clone(),
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

pub(crate) fn repo_owner_ids(repo: &StoredRepository) -> Vec<String> {
    repo.owner_ids()
}
