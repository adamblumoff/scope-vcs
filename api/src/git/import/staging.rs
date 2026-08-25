use crate::config::DEFAULT_GIT_BRANCH;
use scope_domain::landing_file::RepositoryLandingFileMutation;
use scope_domain::repo_config::RepoConfig;
use scope_domain::reviewed_updates::content::{
    ReviewedContentChange, ReviewedUpdateInput, apply_reviewed_update_to_repo,
};
use scope_domain::runs::catalog::RepositoryWorkflowCatalog;
use scope_domain::{
    content::SourceBlob,
    repository::Repository,
    repository::git::{GitHead, GitPackSpan},
};
use scope_domain::{
    error::DomainError, policy::ScopePath, repo_actions::reviewed_update_domain_error,
};

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

pub(crate) fn apply_receive_pack_update(
    repo: &mut Repository,
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
