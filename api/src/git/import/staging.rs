use crate::domain::policy::{ScopePath, Visibility};
use crate::domain::repo_config::{RepoConfig, is_reserved_config_path};
use crate::domain::reviewed_updates::{
    ReviewedContentChange, ReviewedUpdateError, ReviewedUpdateInput, apply_reviewed_update_to_repo,
};
use crate::domain::store::{GitHead, GitSegment, SourceBlob, StoredRepository};
use crate::{config::DEFAULT_GIT_BRANCH, error::ApiError};
use std::collections::BTreeSet;

#[derive(Clone, Debug)]
pub(crate) struct ReceivePackFileChange {
    pub(crate) path: ScopePath,
    pub(crate) content: Option<SourceBlob>,
}

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
pub(crate) struct ReceivePackUpdate {
    pub(crate) branch: String,
    pub(crate) head_oid: String,
    pub(crate) base_git_manifest_key: Option<Option<String>>,
    pub(crate) author_id: String,
    pub(crate) message: String,
    pub(crate) git_head: GitHead,
    pub(crate) git_segment: GitSegment,
    pub(crate) durable_objects: Vec<SourceBlob>,
    pub(crate) changes: Vec<ReceivePackFileChange>,
    pub(crate) previous_config: Option<RepoConfig>,
    pub(crate) base_config_hash: String,
    pub(crate) config: RepoConfig,
}

pub(super) fn apply_receive_pack_update(
    repo: &mut StoredRepository,
    update: ReceivePackUpdate,
) -> Result<(), ApiError> {
    ensure_default_branch(&update.branch)?;
    apply_reviewed_update_to_repo(repo, update.into_reviewed_update())
        .map_err(reviewed_update_error_to_api_error)
}

impl ReceivePackUpdate {
    pub(super) fn into_reviewed_update(self) -> ReviewedUpdateInput {
        ReviewedUpdateInput {
            branch: self.branch,
            author_id: self.author_id,
            message: self.message,
            git_head: self.git_head,
            git_segment: self.git_segment,
            changes: self
                .changes
                .into_iter()
                .map(|change| ReviewedContentChange {
                    path: change.path,
                    content: change.content,
                })
                .collect(),
            previous_config: self.previous_config,
            config: self.config,
        }
    }
}

pub(super) fn receive_pack_update_changes_visibility(
    repo: &StoredRepository,
    previous_config: Option<&RepoConfig>,
    update: &ReceivePackUpdate,
) -> bool {
    if let Some(previous_config) = previous_config {
        return previous_config != &update.config;
    }

    if Visibility::from(update.config.visibility.default_visibility())
        != repo.record.default_visibility
    {
        return true;
    }
    if !update.config.visibility.rules.is_empty() || !update.config.history.rewrites.is_empty() {
        return true;
    }

    let mut paths = repo.live_tree().into_keys().collect::<BTreeSet<_>>();
    for change in &update.changes {
        if change.content.is_some() {
            paths.insert(change.path.clone());
        }
    }

    paths.into_iter().any(|path| {
        !is_reserved_config_path(&path)
            && repo.policy.effective_visibility(&path) != update.config.visibility_for_path(&path)
    })
}

fn reviewed_update_error_to_api_error(error: ReviewedUpdateError) -> ApiError {
    match error {
        ReviewedUpdateError::BadRequest(message) => ApiError::bad_request(message),
        ReviewedUpdateError::Conflict(message) => ApiError::conflict(message),
        ReviewedUpdateError::InvalidPolicy(error) => ApiError::bad_request(error),
    }
}
