use crate::{
    error::ApiError,
    http::responses::{ProjectionPreviewAudience, ProjectionPreviewSource},
    repo_access::ensure_repo_read,
    state::AppState,
};
use scope_domain::{
    policy::Principal, repository::Repository, repository::access::RepositoryActor,
};

pub(crate) fn ensure_projection_preview_access(
    state: &AppState,
    repo: &Repository,
    requester: &Principal,
    audience: ProjectionPreviewAudience,
    source: ProjectionPreviewSource,
) -> Result<(), ApiError> {
    match (audience, source) {
        (ProjectionPreviewAudience::Private, _) => {
            ensure_repo_read(state, repo, requester)?;
            if repo.access_for_principal(requester).actor != RepositoryActor::Public {
                Ok(())
            } else {
                Err(ApiError::forbidden("repo membership required"))
            }
        }
        (ProjectionPreviewAudience::Public, ProjectionPreviewSource::Live) => {
            if repo.access_for_principal(requester).actor != RepositoryActor::Public {
                ensure_repo_read(state, repo, requester)
            } else {
                ensure_repo_read(state, repo, &Principal::public())
            }
        }
    }
}

pub(crate) fn projection_preview_repo(
    repo: &Repository,
    _source: ProjectionPreviewSource,
) -> Result<Repository, ApiError> {
    Ok(repo.clone())
}
