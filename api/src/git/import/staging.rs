use crate::config::DEFAULT_GIT_BRANCH;
use scope_domain::landing_file::RepositoryLandingFileMutation;
use scope_domain::reviewed_updates::{
    ReviewedContentChange, ReviewedUpdateInput, apply_reviewed_update_to_repo,
};
use scope_domain::runs::catalog::RepositoryWorkflowCatalog;
use scope_domain::store::{GitHead, GitPackSpan, SourceBlob, StoredRepository};
use scope_domain::{
    error::DomainError, policy::ScopePath, repo_actions::reviewed_update_domain_error,
};
use scope_domain::{repo_config::RepoConfig, repo_control::is_private_control_path};
use std::collections::BTreeSet;

#[derive(Clone, Debug)]
pub(crate) struct ReceivePackFileChange {
    pub(crate) path: ScopePath,
    pub(crate) content: Option<SourceBlob>,
}

pub(crate) fn ensure_default_branch(branch: &str) -> Result<(), DomainError> {
    let branch = branch.trim();
    match branch {
        DEFAULT_GIT_BRANCH => Ok(()),
        value if value == format!("refs/heads/{DEFAULT_GIT_BRANCH}") => Ok(()),
        value if value.starts_with("refs/tags/") => Err(DomainError::invalid_input(
            "tags are not supported by Scope pushes",
        )),
        _ => Err(DomainError::invalid_input(
            "Scope accepts pushes only to the default branch refs/heads/main",
        )),
    }
}

#[derive(Clone, Debug)]
pub(crate) struct ReceivePackUpdate {
    pub(crate) branch: String,
    pub(crate) head_oid: String,
    pub(crate) base_git_manifest_ref: Option<Option<scope_domain::content_ref::ContentRef>>,
    pub(crate) author_id: String,
    pub(crate) message: String,
    pub(crate) git_head: GitHead,
    pub(crate) git_pack_span: GitPackSpan,
    pub(crate) durable_objects: Vec<SourceBlob>,
    pub(crate) workflow_catalog: RepositoryWorkflowCatalog,
    pub(crate) landing_file_mutation: RepositoryLandingFileMutation,
    pub(crate) changes: Vec<ReceivePackFileChange>,
    pub(crate) previous_config: Option<RepoConfig>,
    pub(crate) base_config_hash: String,
    pub(crate) config: RepoConfig,
}

pub(super) fn apply_receive_pack_update(
    repo: &mut StoredRepository,
    update: ReceivePackUpdate,
) -> Result<(), DomainError> {
    ensure_default_branch(&update.branch)?;
    apply_reviewed_update_to_repo(repo, update.into_reviewed_update())
        .map_err(reviewed_update_domain_error)
}

impl ReceivePackUpdate {
    pub(crate) fn into_reviewed_update(self) -> ReviewedUpdateInput {
        ReviewedUpdateInput {
            branch: self.branch,
            author_id: self.author_id,
            message: self.message,
            git_head: self.git_head,
            git_pack_span: self.git_pack_span,
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

    if update.config.visibility.default_visibility()
        != repo.repo_config.visibility.default_visibility()
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
        !is_private_control_path(&path)
            && repo.policy.effective_visibility(&path) != update.config.visibility_for_path(&path)
    })
}
