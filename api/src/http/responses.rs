mod projections;
mod repo_collaboration;
mod requests;

pub(crate) use projections::*;
pub(crate) use repo_collaboration::*;
pub(crate) use requests::*;
use scope_api_contract::{
    DeviceLoginStatus, FileChangeKind, FirstPushTokenResponse, GitOid, GitPushTokenResponse,
    RepoInitResponse, RepoLifecycleState, RepoRequestPermissionsResponse, RepoSummaryResponse,
    RepositoryAccessResponse, RequestActorSummaryResponse, RequestRevisionCommitResponse,
    SessionIdentity, UserResponse, Visibility,
};

use crate::{config::DEFAULT_GIT_BRANCH, error::ApiError};
use scope_domain::commit_history::{CommitHistoryCommit, CommitHistoryView};
use scope_domain::policy::ScopePath;
use scope_domain::store::{
    FirstPushToken, GitPushToken, RepoLifecycleState as DomainRepoLifecycleState, RepositoryAccess,
    RepositoryActor, StoredRepository, UserAccount,
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

#[derive(Clone, Copy, Debug, Serialize)]
#[serde(rename_all = "kebab-case")]
#[cfg_attr(feature = "type-export", derive(ts_rs::TS))]
#[cfg_attr(feature = "type-export", ts(rename_all = "kebab-case"))]
pub(crate) enum RepositoryRunState {
    Queued,
    Leased,
    Running,
    Succeeded,
    Failed,
    Canceled,
    Lost,
}

impl From<scope_domain::runs::run::RunState> for RepositoryRunState {
    fn from(state: scope_domain::runs::run::RunState) -> Self {
        use scope_domain::runs::run::RunState;
        match state {
            RunState::Queued => Self::Queued,
            RunState::Leased => Self::Leased,
            RunState::Running => Self::Running,
            RunState::Succeeded => Self::Succeeded,
            RunState::Failed => Self::Failed,
            RunState::Canceled => Self::Canceled,
            RunState::Lost => Self::Lost,
        }
    }
}

#[derive(Debug, Serialize)]
#[cfg_attr(feature = "type-export", derive(ts_rs::TS))]
pub(crate) struct RepositoryRunSummaryResponse {
    pub(crate) id: String,
    pub(crate) workflow_name: String,
    pub(crate) git_oid: String,
    pub(crate) desired_runner: Option<String>,
    pub(crate) state: RepositoryRunState,
    pub(crate) cancellation_requested: bool,
    pub(crate) created_at_unix: u64,
    pub(crate) updated_at_unix: u64,
    pub(crate) completed_at_unix: Option<u64>,
    pub(crate) can_cancel: bool,
    pub(crate) can_retry: bool,
}

#[derive(Clone, Copy, Debug, Serialize)]
#[serde(rename_all = "kebab-case")]
#[cfg_attr(feature = "type-export", derive(ts_rs::TS))]
#[cfg_attr(feature = "type-export", ts(rename_all = "kebab-case"))]
pub(crate) enum RepositoryRunAttemptState {
    Leased,
    Running,
    Succeeded,
    Failed,
    Canceled,
    Lost,
}

impl From<scope_domain::runs::run::AttemptState> for RepositoryRunAttemptState {
    fn from(state: scope_domain::runs::run::AttemptState) -> Self {
        use scope_domain::runs::run::AttemptState;
        match state {
            AttemptState::Leased => Self::Leased,
            AttemptState::Running => Self::Running,
            AttemptState::Succeeded => Self::Succeeded,
            AttemptState::Failed => Self::Failed,
            AttemptState::Canceled => Self::Canceled,
            AttemptState::Lost => Self::Lost,
        }
    }
}

#[derive(Clone, Copy, Debug, Serialize)]
#[serde(rename_all = "kebab-case")]
#[cfg_attr(feature = "type-export", derive(ts_rs::TS))]
#[cfg_attr(feature = "type-export", ts(rename_all = "kebab-case"))]
pub(crate) enum RepositoryRunStepState {
    Pending,
    Running,
    Succeeded,
    Failed,
    Canceled,
    Lost,
    Skipped,
}

impl From<scope_domain::runs::run::StepState> for RepositoryRunStepState {
    fn from(state: scope_domain::runs::run::StepState) -> Self {
        use scope_domain::runs::run::StepState;
        match state {
            StepState::Pending => Self::Pending,
            StepState::Running => Self::Running,
            StepState::Succeeded => Self::Succeeded,
            StepState::Failed => Self::Failed,
            StepState::Canceled => Self::Canceled,
            StepState::Lost => Self::Lost,
            StepState::Skipped => Self::Skipped,
        }
    }
}

#[derive(Clone, Debug, Serialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
#[cfg_attr(feature = "type-export", derive(ts_rs::TS))]
#[cfg_attr(feature = "type-export", ts(tag = "kind", rename_all = "kebab-case"))]
pub(crate) enum RepositoryRunTerminalReason {
    StepFailed { step_index: u32, exit_code: i32 },
    TimedOut { step_index: Option<u32> },
    Canceled { step_index: Option<u32> },
    RunnerLost { step_index: Option<u32> },
    RunnerSetupFailed { exit_code: i32, message: String },
}

impl From<scope_domain::runs::run::AttemptTerminalReason> for RepositoryRunTerminalReason {
    fn from(reason: scope_domain::runs::run::AttemptTerminalReason) -> Self {
        use scope_domain::runs::run::AttemptTerminalReason;
        match reason {
            AttemptTerminalReason::StepFailed {
                step_index,
                exit_code,
            } => Self::StepFailed {
                step_index,
                exit_code,
            },
            AttemptTerminalReason::TimedOut { step_index } => Self::TimedOut { step_index },
            AttemptTerminalReason::Canceled { step_index } => Self::Canceled { step_index },
            AttemptTerminalReason::RunnerLost { step_index } => Self::RunnerLost { step_index },
            AttemptTerminalReason::RunnerSetupFailed { exit_code, message } => {
                Self::RunnerSetupFailed { exit_code, message }
            }
        }
    }
}

#[derive(Debug, Serialize)]
#[cfg_attr(feature = "type-export", derive(ts_rs::TS))]
pub(crate) struct RepositoryRunStepResponse {
    pub(crate) index: u32,
    pub(crate) name: String,
    pub(crate) command: String,
    pub(crate) state: RepositoryRunStepState,
    pub(crate) started_at_unix: Option<u64>,
    pub(crate) completed_at_unix: Option<u64>,
    pub(crate) exit_code: Option<i32>,
}

#[derive(Debug, Serialize)]
#[cfg_attr(feature = "type-export", derive(ts_rs::TS))]
pub(crate) struct RepositoryRunAttemptResponse {
    pub(crate) id: String,
    pub(crate) job_key: String,
    pub(crate) runner_id: String,
    pub(crate) runner_name: String,
    pub(crate) state: RepositoryRunAttemptState,
    pub(crate) created_at_unix: u64,
    pub(crate) started_at_unix: Option<u64>,
    pub(crate) completed_at_unix: Option<u64>,
    pub(crate) terminal_reason: Option<RepositoryRunTerminalReason>,
    pub(crate) steps: Vec<RepositoryRunStepResponse>,
}

#[derive(Clone, Copy, Debug, Serialize)]
#[serde(rename_all = "lowercase")]
#[cfg_attr(feature = "type-export", derive(ts_rs::TS))]
#[cfg_attr(feature = "type-export", ts(rename_all = "lowercase"))]
pub(crate) enum RepositoryRunnerState {
    Online,
    Offline,
    Disabled,
}

#[derive(Debug, Serialize)]
#[cfg_attr(feature = "type-export", derive(ts_rs::TS))]
pub(crate) struct RepositoryRunnerResponse {
    pub(crate) id: String,
    pub(crate) name: String,
    pub(crate) version: String,
    pub(crate) state: RepositoryRunnerState,
    pub(crate) last_seen_at_unix: Option<u64>,
}

#[derive(Debug, Serialize)]
#[cfg_attr(feature = "type-export", derive(ts_rs::TS))]
pub(crate) struct RepositoryOperationsResponse {
    pub(crate) runs: Vec<RepositoryRunSummaryResponse>,
    pub(crate) runners: Vec<RepositoryRunnerResponse>,
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
pub(crate) struct RepositoryRunDetailResponse {
    pub(crate) run: RepositoryRunSummaryResponse,
    pub(crate) attempts: Vec<RepositoryRunAttemptResponse>,
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
pub(crate) struct CommitHistoryRequest {
    pub(crate) audience: Option<ProjectionPreviewAudience>,
}

#[derive(Debug, Deserialize)]
#[cfg_attr(feature = "type-export", derive(ts_rs::TS))]
pub(crate) struct CommitFileDiffRequest {
    pub(crate) audience: Option<ProjectionPreviewAudience>,
    pub(crate) path: String,
}

#[derive(Debug, Deserialize)]
#[cfg_attr(feature = "type-export", derive(ts_rs::TS))]
pub(crate) struct RequestFileDiffRequest {
    pub(crate) path: String,
}

#[derive(Debug, Serialize)]
#[cfg_attr(feature = "type-export", derive(ts_rs::TS))]
pub(crate) struct RequestRevisionCommitFilesResponse {
    pub(crate) revision_id: String,
    pub(crate) commit: RequestRevisionCommitResponse,
    pub(crate) files: Vec<CommitFileResponse>,
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
pub(crate) struct CommitHistoryResponse {
    pub(crate) audience: ProjectionPreviewAudience,
    pub(crate) repo_id: String,
    pub(crate) view_key: String,
    pub(crate) generation: String,
    pub(crate) commits: Vec<CommitSummaryResponse>,
}

#[derive(Debug, Serialize)]
#[cfg_attr(feature = "type-export", derive(ts_rs::TS))]
pub(crate) struct CommitSummaryResponse {
    pub(crate) projected_id: String,
    pub(crate) logical_commit_id: String,
    pub(crate) parent_projected_id: Option<String>,
    pub(crate) author: Option<String>,
    pub(crate) message: String,
    pub(crate) change_count: usize,
}

#[derive(Debug, Serialize)]
#[cfg_attr(feature = "type-export", derive(ts_rs::TS))]
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
#[cfg_attr(feature = "type-export", derive(ts_rs::TS))]
pub(crate) struct CommitFileResponse {
    pub(crate) path: String,
    pub(crate) kind: FileChangeKind,
    pub(crate) old_mode: Option<String>,
    pub(crate) new_mode: Option<String>,
    pub(crate) old_oid: Option<String>,
    pub(crate) new_oid: Option<String>,
    pub(crate) visibility: Visibility,
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
    let secret = if status == scope_domain::store::FirstPushTokenStatus::Active {
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

pub(crate) fn commit_history_response(
    audience: ProjectionPreviewAudience,
    view: CommitHistoryView,
) -> CommitHistoryResponse {
    CommitHistoryResponse {
        audience,
        repo_id: view.repo_id,
        view_key: view.view_key,
        generation: view.generation,
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
    file: &scope_domain::commit_history::CommitHistoryFile,
) -> CommitFileResponse {
    CommitFileResponse {
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
