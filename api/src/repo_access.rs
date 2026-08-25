use crate::{error::ApiError, state::AppState};
use scope_domain::{
    policy::{Principal, ScopePath},
    projection_views::has_visible_projected_non_control_files,
    repository::access::{RepositoryAccess, RepositoryActor},
    repository::{RepoLifecycleState, Repository},
};

pub(crate) async fn find_repo(
    state: &AppState,
    owner: &str,
    name: &str,
) -> Result<Repository, ApiError> {
    state
        .metadata
        .repositories()
        .repository(owner, name)
        .await?
        .ok_or_else(|| ApiError::not_found(format!("repo {owner}/{name} not found")))
}

pub(crate) fn ensure_repo_read(
    state: &AppState,
    repo: &Repository,
    principal: &Principal,
) -> Result<(), ApiError> {
    let access = repo.access_for_principal(principal);
    let readable = if access.actor == RepositoryActor::Public {
        repo.record.lifecycle_state == RepoLifecycleState::Ready
            && has_visible_projected_non_control_files(repo, principal)
    } else {
        can_read_path(state, repo, principal, &ScopePath::root())?
    };

    if readable {
        Ok(())
    } else {
        Err(ApiError::not_found(format!(
            "repo {} not found",
            repo.record.id
        )))
    }
}

pub(crate) fn access_for_principal(
    _state: &AppState,
    repo: &Repository,
    principal: &Principal,
) -> Result<RepositoryAccess, ApiError> {
    Ok(repo.access_for_principal(principal))
}

pub(crate) fn can_read_path(
    _state: &AppState,
    repo: &Repository,
    principal: &Principal,
    path: &ScopePath,
) -> Result<bool, ApiError> {
    Ok(repo.can_read_path(principal, path))
}
