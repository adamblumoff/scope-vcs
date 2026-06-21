use crate::domain::policy::{Principal, ScopePath, Visibility};
use crate::domain::projection::{FileChange, Projection, project_graph};
use crate::domain::store::{
    AppCatalog, FirstPushToken, FirstPushTokenStatus, GitPushToken, PendingImport,
    RepoPublicationState, RepoRole, StagedFileChangeKind, StagedRepoUpdate, StoredRepository,
    UserAccount,
};
use crate::{
    auth::tokens::ensure_owner_setup_access_in_catalog,
    config::{DEFAULT_GIT_BRANCH, FIRST_PUSH_TOKEN_TTL_SECS},
    error::ApiError,
    object_store::{ObjectStore, source_blob_text},
    state::graph_has_file,
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

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
pub(crate) struct AccountSessionResponse {
    pub(crate) identity: Option<SessionIdentity>,
    pub(crate) user: Option<UserResponse>,
}

#[derive(Debug, Serialize)]
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
pub(crate) struct SessionResponse {
    pub(crate) identity: Option<SessionIdentity>,
    pub(crate) repo: SessionRepo,
    pub(crate) principal_id: String,
    pub(crate) capabilities: SessionCapabilities,
}

#[derive(Debug, Serialize)]
pub(crate) struct SessionIdentity {
    pub(crate) pairwise_sub: String,
    pub(crate) email: Option<String>,
    pub(crate) email_verified: bool,
}

#[derive(Debug, Serialize)]
pub(crate) struct SessionRepo {
    pub(crate) id: String,
    pub(crate) publication_state: RepoPublicationState,
    pub(crate) role: Option<RepoRole>,
}

#[derive(Debug, Serialize)]
pub(crate) struct SessionCapabilities {
    pub(crate) read: bool,
    pub(crate) write: bool,
}

#[derive(Debug, Serialize)]
pub(crate) struct RepoSummaryResponse {
    pub(crate) id: String,
    pub(crate) owner_handle: String,
    pub(crate) name: String,
    pub(crate) lifecycle_state: RepoPublicationState,
    pub(crate) default_visibility: Visibility,
    pub(crate) role: Option<RepoRole>,
    pub(crate) staged_update_pending: bool,
}

#[derive(Debug, Serialize)]
pub(crate) struct CreateRepoResponse {
    pub(crate) repo: RepoSummaryResponse,
    pub(crate) setup: RepoSetupResponse,
}

#[derive(Debug, Serialize)]
pub(crate) struct DeleteRepoResponse {
    pub(crate) id: String,
    pub(crate) deleted: bool,
}

#[derive(Debug, Serialize)]
pub(crate) struct ProjectionResponse {
    pub(crate) repo_id: String,
    pub(crate) principal_id: String,
    pub(crate) commits: Vec<ProjectedCommitResponse>,
}

#[derive(Debug, Serialize)]
pub(crate) struct ProjectedCommitResponse {
    pub(crate) projected_id: String,
    pub(crate) logical_commit_id: String,
    pub(crate) parent_projected_id: Option<String>,
    pub(crate) author: Option<String>,
    pub(crate) message: String,
    pub(crate) synthetic: bool,
    pub(crate) changes: Vec<ProjectedChangeResponse>,
}

#[derive(Debug, Serialize)]
pub(crate) struct ProjectedChangeResponse {
    pub(crate) path: ScopePath,
    pub(crate) new_content: Option<String>,
}

#[derive(Debug, Serialize)]
pub(crate) struct RepoSetupResponse {
    pub(crate) repo: RepoSummaryResponse,
    pub(crate) git_remote_path: String,
    pub(crate) remote_name: &'static str,
    pub(crate) push_branch: &'static str,
    pub(crate) push_enabled: bool,
    pub(crate) token: Option<FirstPushTokenResponse>,
    pub(crate) push_token: Option<GitPushTokenResponse>,
}

#[derive(Debug, Serialize)]
pub(crate) struct FirstPushTokenResponse {
    pub(crate) status: FirstPushTokenStatus,
    pub(crate) created_at_unix: u64,
    pub(crate) expires_at_unix: u64,
    pub(crate) used_at_unix: Option<u64>,
    pub(crate) secret: Option<String>,
}

#[derive(Debug, Serialize)]
pub(crate) struct GitPushTokenResponse {
    pub(crate) created_at_unix: u64,
    pub(crate) secret: Option<String>,
}

#[derive(Debug, Serialize)]
pub(crate) struct RepoFileResponse {
    pub(crate) path: String,
    pub(crate) oid: String,
    pub(crate) tracked: bool,
    pub(crate) visibility: Visibility,
}

#[derive(Debug, Serialize)]
pub(crate) struct PendingImportReviewResponse {
    pub(crate) publication_state: RepoPublicationState,
    pub(crate) default_visibility: Visibility,
    pub(crate) files: Vec<RepoFileResponse>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct UpdateFileVisibilityRequest {
    pub(crate) paths: Vec<String>,
    pub(crate) visibility: Visibility,
}

#[derive(Debug, Deserialize)]
pub(crate) struct UpdateRepoSettingsRequest {
    pub(crate) include_ignored_files: bool,
    pub(crate) review_pushes_before_applying: bool,
}

#[derive(Debug, Deserialize)]
pub(crate) struct UpdateStagedFileVisibilityRequest {
    pub(crate) paths: Vec<String>,
    pub(crate) visibility: Visibility,
}

#[derive(Debug, Serialize)]
pub(crate) struct StagedUpdateResponse {
    pub(crate) id: String,
    pub(crate) branch: String,
    pub(crate) base_live_commit_id: Option<String>,
    pub(crate) message: String,
    pub(crate) files: Vec<StagedFileResponse>,
}

#[derive(Debug, Serialize)]
pub(crate) struct StagedFileResponse {
    pub(crate) path: String,
    pub(crate) kind: StagedFileChangeKind,
    pub(crate) old_oid: Option<String>,
    pub(crate) new_oid: Option<String>,
    pub(crate) visibility: Visibility,
}

#[derive(Debug, Deserialize)]
pub(crate) struct CreateRepoRequest {
    pub(crate) name: String,
    pub(crate) visibility: Option<Visibility>,
}

pub(crate) fn repo_summary(
    catalog: &AppCatalog,
    repo: &StoredRepository,
    user_id: &str,
) -> Option<RepoSummaryResponse> {
    let principal = Principal {
        id: user_id.to_string(),
        kind: crate::domain::policy::PrincipalKind::User,
    };
    if !catalog.can_read_path(repo, &principal, &ScopePath::root()) {
        return None;
    }

    let role = catalog.role_for_principal(repo, &principal)?;

    Some(RepoSummaryResponse {
        id: repo.record.id.clone(),
        owner_handle: repo.record.owner_handle.clone(),
        name: repo.record.name.clone(),
        lifecycle_state: repo.record.publication_state,
        default_visibility: repo.record.default_visibility,
        role: Some(role),
        staged_update_pending: role == RepoRole::Owner && repo.staged_update.is_some(),
    })
}

pub(crate) fn repo_setup_response(
    catalog: &AppCatalog,
    repo: &StoredRepository,
    user_id: &str,
    now_unix: u64,
    secret: Option<String>,
    push_secret: Option<String>,
) -> Result<RepoSetupResponse, ApiError> {
    ensure_owner_setup_access_in_catalog(catalog, repo, user_id)?;
    let repo = repo_summary(catalog, repo, user_id)
        .ok_or_else(|| ApiError::internal_message("setup repository is not readable"))?;
    let token = catalog
        .repositories
        .get(&repo.id)
        .and_then(|stored| stored.first_push_token.as_ref())
        .map(|stored_token| first_push_token_response(stored_token, now_unix, secret));
    let push_token = catalog
        .repositories
        .get(&repo.id)
        .and_then(|stored| stored.git_push_token.as_ref())
        .map(|stored_token| git_push_token_response(stored_token, push_secret));

    Ok(RepoSetupResponse {
        git_remote_path: format!("/git/{}/{}", repo.owner_handle, repo.name),
        remote_name: "scope",
        push_branch: DEFAULT_GIT_BRANCH,
        push_enabled: true,
        repo,
        token,
        push_token,
    })
}

pub(crate) fn first_push_token_response(
    token: &FirstPushToken,
    now_unix: u64,
    secret: Option<String>,
) -> FirstPushTokenResponse {
    let status = first_push_token_status_at(token, now_unix);
    let secret = if status == FirstPushTokenStatus::Active {
        secret
    } else {
        None
    };

    FirstPushTokenResponse {
        status,
        created_at_unix: token.created_at_unix,
        expires_at_unix: first_push_token_expires_at(token),
        used_at_unix: token.used_at_unix,
        secret,
    }
}

pub(crate) fn first_push_token_status_at(
    token: &FirstPushToken,
    now_unix: u64,
) -> FirstPushTokenStatus {
    if token.used_at_unix.is_some() {
        FirstPushTokenStatus::Used
    } else if now_unix >= first_push_token_expires_at(token) {
        FirstPushTokenStatus::Expired
    } else {
        FirstPushTokenStatus::Active
    }
}

pub(crate) fn first_push_token_expires_at(token: &FirstPushToken) -> u64 {
    token
        .created_at_unix
        .saturating_add(FIRST_PUSH_TOKEN_TTL_SECS)
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

pub(crate) fn staged_update_response(update: &StagedRepoUpdate) -> StagedUpdateResponse {
    StagedUpdateResponse {
        id: update.id.clone(),
        branch: update.branch.clone(),
        base_live_commit_id: update.base_live_commit_id.clone(),
        message: update.message.clone(),
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

pub(crate) fn pending_import_has_file(repo: &StoredRepository, path: &ScopePath) -> bool {
    repo.pending_import.as_ref().is_some_and(|pending| {
        pending.files.iter().any(|file| {
            pending_scope_path(&file.path)
                .map(|pending_path| pending_path.as_str() == path.as_str())
                .unwrap_or(false)
        })
    })
}

pub(crate) fn repo_has_file_for_review(repo: &StoredRepository, path: &ScopePath) -> bool {
    if repo.record.publication_state == RepoPublicationState::PendingPublish {
        pending_import_has_file(repo, path)
    } else {
        graph_has_file(repo, path)
    }
}

pub(crate) fn repo_owner_ids(repo: &StoredRepository) -> Vec<String> {
    let mut owner_ids = repo
        .memberships
        .iter()
        .filter(|membership| membership.role == RepoRole::Owner)
        .map(|membership| membership.user_id.clone())
        .collect::<Vec<_>>();
    if !owner_ids.contains(&repo.record.owner_user_id) {
        owner_ids.push(repo.record.owner_user_id.clone());
    }
    owner_ids.sort();
    owner_ids.dedup();
    owner_ids
}

pub(crate) fn projection_response(
    store: &dyn ObjectStore,
    projection: Projection,
) -> Result<ProjectionResponse, ApiError> {
    Ok(ProjectionResponse {
        repo_id: projection.repo_id,
        principal_id: projection.principal_id,
        commits: projection
            .commits
            .into_iter()
            .map(|commit| {
                let changes = commit
                    .changes
                    .into_iter()
                    .map(|change| {
                        Ok(ProjectedChangeResponse {
                            path: change.path,
                            new_content: change
                                .new_content
                                .as_ref()
                                .map(|blob| source_blob_text(store, blob))
                                .transpose()?,
                        })
                    })
                    .collect::<Result<Vec<_>, ApiError>>()?;
                Ok(ProjectedCommitResponse {
                    projected_id: commit.projected_id,
                    logical_commit_id: commit.logical_commit_id,
                    parent_projected_id: commit.parent_projected_id,
                    author: commit.author,
                    message: commit.message,
                    synthetic: commit.synthetic,
                    changes,
                })
            })
            .collect::<Result<Vec<_>, ApiError>>()?,
    })
}

pub(crate) fn projected_files(
    repo: &StoredRepository,
    principal: &Principal,
) -> Result<Vec<RepoFileResponse>, ApiError> {
    let projection = project_graph(&repo.policy, &repo.graph, principal);
    let mut tree = BTreeMap::new();
    for change in projection
        .commits
        .iter()
        .flat_map(|commit| commit.changes.iter())
    {
        match &change.new_content {
            Some(blob) => {
                tree.insert(change.path.clone(), blob.git_oid.clone());
            }
            None => {
                tree.remove(&change.path);
            }
        }
    }

    Ok(tree
        .into_iter()
        .map(|(path, oid)| RepoFileResponse {
            visibility: repo.policy.effective_visibility(&path),
            path: path.as_str().to_string(),
            oid,
            tracked: true,
        })
        .collect())
}

pub(crate) fn pending_import_files(
    repo: &StoredRepository,
    principal: &Principal,
) -> Result<Vec<RepoFileResponse>, ApiError> {
    let Some(pending) = repo.pending_import.as_ref() else {
        return Ok(Vec::new());
    };
    let mut files = Vec::new();
    for file in &pending.files {
        let path = pending_scope_path(&file.path)?;
        if !repo.policy.can_read(principal, &path) {
            continue;
        }
        files.push(RepoFileResponse {
            path: path.as_str().to_string(),
            oid: file.oid.clone(),
            tracked: true,
            visibility: repo.policy.effective_visibility(&path),
        });
    }
    files.sort_by(|left, right| left.path.cmp(&right.path));
    Ok(files)
}

pub(crate) fn files_for_visibility_update(
    repo: &StoredRepository,
    principal: &Principal,
) -> Result<Vec<RepoFileResponse>, ApiError> {
    if repo.record.publication_state == RepoPublicationState::PendingPublish {
        pending_import_files(repo, principal)
    } else {
        projected_files(repo, principal)
    }
}

pub(crate) fn pending_import_changes(pending: &PendingImport) -> Vec<FileChange> {
    pending
        .files
        .iter()
        .map(|file| FileChange {
            path: pending_scope_path(&file.path)
                .expect("pending import paths were validated before persistence"),
            old_content: None,
            new_content: Some(file.blob.clone()),
        })
        .collect()
}

pub(crate) fn pending_scope_path(path: &str) -> Result<ScopePath, ApiError> {
    let path = if path.starts_with('/') {
        path.to_string()
    } else {
        format!("/{path}")
    };
    ScopePath::parse(path).map_err(ApiError::bad_request)
}
