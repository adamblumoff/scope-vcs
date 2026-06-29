use crate::{
    domain::{
        policy::Principal,
        projection_views::repo_for_projection_preview,
        store::{RepositoryActor, StoredRepository},
    },
    error::ApiError,
    http::responses::{ProjectionPreviewAudience, ProjectionPreviewSource},
    state::{AppState, ensure_owner, ensure_repo_read},
};

pub(crate) fn ensure_projection_preview_access(
    state: &AppState,
    repo: &StoredRepository,
    requester: &Principal,
    audience: ProjectionPreviewAudience,
    source: ProjectionPreviewSource,
) -> Result<(), ApiError> {
    match (audience, source) {
        (ProjectionPreviewAudience::Owner, _) => {
            ensure_repo_read(state, repo, requester)?;
            ensure_review_preview_access(state, repo, requester, source)?;
            if repo.access_for_principal(requester).actor != RepositoryActor::Public {
                Ok(())
            } else {
                Err(ApiError::forbidden("repo membership required"))
            }
        }
        (ProjectionPreviewAudience::Public, ProjectionPreviewSource::Review) => {
            ensure_repo_read(state, repo, requester)?;
            ensure_review_preview_access(state, repo, requester, source)
        }
        (ProjectionPreviewAudience::Public, ProjectionPreviewSource::Live) => {
            if repo.access_for_principal(requester).actor == RepositoryActor::Owner {
                ensure_repo_read(state, repo, requester)
            } else {
                ensure_repo_read(state, repo, &Principal::public())
            }
        }
    }
}

fn ensure_review_preview_access(
    state: &AppState,
    repo: &StoredRepository,
    requester: &Principal,
    source: ProjectionPreviewSource,
) -> Result<(), ApiError> {
    if source != ProjectionPreviewSource::Review {
        return Ok(());
    }

    if repo.has_pending_import_review() {
        return ensure_owner(state, repo, requester);
    }

    let access = repo.access_for_principal(requester);
    if access.actor == RepositoryActor::Owner
        || access.can_apply_changes
        || access.can_change_file_visibility
    {
        Ok(())
    } else {
        Err(ApiError::forbidden("review permission required"))
    }
}

pub(crate) fn projection_preview_repo(
    repo: &StoredRepository,
    source: ProjectionPreviewSource,
) -> Result<StoredRepository, ApiError> {
    repo_for_projection_preview(repo, source.into())
}
