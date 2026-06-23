use crate::{
    domain::{
        policy::Principal,
        repo_actions::promote_pending_import,
        store::{RepoPublicationState, RepoRole, StoredRepository},
    },
    error::ApiError,
    http::responses::{ProjectionPreviewAudience, ProjectionPreviewSource},
    state::{AppState, ensure_owner, ensure_repo_read, role_for_principal},
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
            ensure_owner(state, repo, requester)
        }
        (ProjectionPreviewAudience::Public, ProjectionPreviewSource::Review) => {
            ensure_repo_read(state, repo, requester)?;
            ensure_owner(state, repo, requester)
        }
        (ProjectionPreviewAudience::Public, ProjectionPreviewSource::Live) => {
            if role_for_principal(state, repo, requester)? == Some(RepoRole::Owner) {
                ensure_repo_read(state, repo, requester)
            } else {
                ensure_repo_read(state, repo, &Principal::public())
            }
        }
    }
}

pub(crate) fn projection_preview_repo(
    repo: &StoredRepository,
    source: ProjectionPreviewSource,
) -> Result<StoredRepository, ApiError> {
    let mut preview = repo.clone();
    match source {
        ProjectionPreviewSource::Live => Ok(preview),
        ProjectionPreviewSource::Review => {
            if preview.record.publication_state == RepoPublicationState::PendingPublish {
                promote_pending_import(&mut preview)?;
            } else if let Some(staged_update) = preview.staged_update.clone() {
                crate::git::import::apply_receive_pack_update(&mut preview, staged_update)?;
            } else {
                return Err(ApiError::bad_request("repo has no pending review"));
            }
            Ok(preview)
        }
    }
}
