use crate::{error::ApiError, state::AppState};
use scope_domain::{
    policy::{Principal, ScopePath},
    projection_views::has_visible_projected_history,
    store::{RepoPublicationState, RepositoryAccess, StoredRepository},
};

pub(crate) async fn find_repo(
    state: &AppState,
    owner: &str,
    name: &str,
) -> Result<StoredRepository, ApiError> {
    state
        .metadata
        .repositories()
        .repository(owner, name)
        .await?
        .ok_or_else(|| ApiError::not_found(format!("repo {owner}/{name} not found")))
}

pub(crate) fn ensure_repo_read(
    state: &AppState,
    repo: &StoredRepository,
    principal: &Principal,
) -> Result<(), ApiError> {
    if can_read_path(state, repo, principal, &ScopePath::root())?
        || (repo.record.publication_state == RepoPublicationState::Published
            && has_visible_projected_history(repo, principal))
    {
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
    repo: &StoredRepository,
    principal: &Principal,
) -> Result<RepositoryAccess, ApiError> {
    Ok(repo.access_for_principal(principal))
}

pub(crate) fn can_read_path(
    _state: &AppState,
    repo: &StoredRepository,
    principal: &Principal,
    path: &ScopePath,
) -> Result<bool, ApiError> {
    Ok(repo.can_read_path(principal, path))
}
