use crate::domain::policy::ScopePath;
use crate::domain::staged_updates::{
    StagedContentChange, StagedUpdateError, StagedUpdateInput, stage_staged_update,
};
use crate::domain::store::{SourceBlob, StagedRepoUpdate, StoredRepository};
use crate::{config::DEFAULT_GIT_BRANCH, error::ApiError};

#[derive(Clone, Debug)]
pub(crate) struct ReceivePackFileChange {
    pub(crate) path: ScopePath,
    pub(crate) content: Option<SourceBlob>,
}

#[allow(dead_code)]
pub(crate) fn ensure_default_branch(branch: &str) -> Result<(), ApiError> {
    let branch = branch.trim();
    match branch {
        DEFAULT_GIT_BRANCH => Ok(()),
        value if value == format!("refs/heads/{DEFAULT_GIT_BRANCH}") => Ok(()),
        value if value.starts_with("refs/tags/") => Err(ApiError::bad_request(
            "tags are not supported by Scope pushes",
        )),
        _ => Err(ApiError::bad_request(
            "Scope accepts pushes only to the default branch refs/heads/main",
        )),
    }
}

#[derive(Clone, Debug)]
#[allow(dead_code)]
pub(crate) struct ReceivePackUpdate {
    pub(crate) branch: String,
    pub(crate) author_id: String,
    pub(crate) message: String,
    pub(crate) git_snapshot: SourceBlob,
    pub(crate) uploaded_blobs: Vec<SourceBlob>,
    pub(crate) changes: Vec<ReceivePackFileChange>,
}

// Handoff point for a real post-publish receive-pack parser. This stays
// private so JSON never becomes the product push flow.
#[allow(dead_code)]
#[cfg(test)]
pub(crate) fn stage_receive_pack_update(
    repo: &mut StoredRepository,
    update: ReceivePackUpdate,
) -> Result<Option<StagedRepoUpdate>, ApiError> {
    stage_receive_pack_update_for_access(repo, update, true)
}

pub(super) fn stage_receive_pack_update_for_access(
    repo: &mut StoredRepository,
    update: ReceivePackUpdate,
    can_apply_changes: bool,
) -> Result<Option<StagedRepoUpdate>, ApiError> {
    ensure_default_branch(&update.branch)?;
    stage_staged_update(
        repo,
        StagedUpdateInput {
            branch: update.branch,
            author_id: update.author_id,
            message: update.message,
            git_snapshot: update.git_snapshot,
            changes: update
                .changes
                .into_iter()
                .map(|change| StagedContentChange {
                    path: change.path,
                    content: change.content,
                })
                .collect(),
        },
        can_apply_changes,
    )
    .map_err(staged_update_error_to_api_error)
}

fn staged_update_error_to_api_error(error: StagedUpdateError) -> ApiError {
    match error {
        StagedUpdateError::BadRequest(message) => ApiError::bad_request(message),
        StagedUpdateError::Conflict(message) => ApiError::conflict(message),
        StagedUpdateError::InvalidPolicy(error) => ApiError::bad_request(error),
    }
}
