use super::diff::staged_file_line_diff;
use crate::domain::policy::{ScopePath, Visibility, VisibilityRule};
use crate::domain::projection::{
    AuthorVisibility, FileChange, FileVisibilityChange, LogicalCommit, MixedCommitPolicy,
};
use crate::domain::store::{
    LineDiff, RepoPublicationState, SourceBlob, StagedFileChange, StagedFileChangeKind,
    StagedRepoUpdate, StoredRepository,
};
use crate::{
    config::DEFAULT_GIT_BRANCH, error::ApiError, http::responses::repo_owner_ids,
    object_store::ObjectStore, state::live_tree,
};

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
    let store = crate::object_store::MemoryObjectStore::new();
    stage_receive_pack_update_with_store(repo, update, &store)
}

pub(super) fn stage_receive_pack_update_with_store(
    repo: &mut StoredRepository,
    update: ReceivePackUpdate,
    store: &dyn ObjectStore,
) -> Result<Option<StagedRepoUpdate>, ApiError> {
    ensure_default_branch(&update.branch)?;
    if repo.record.publication_state != RepoPublicationState::Published {
        return Err(ApiError::conflict("repo must be published before push"));
    }
    if update.changes.is_empty() {
        return Err(ApiError::bad_request(
            "receive-pack update must include file changes",
        ));
    }
    if repo.staged_update.is_some() {
        return Err(ApiError::conflict("a staged update is already pending"));
    }

    let staged_update = build_staged_receive_pack_update(repo, update, store)?;
    if repo.settings.review_pushes_before_applying {
        repo.staged_update = Some(staged_update.clone());
        Ok(Some(staged_update))
    } else {
        apply_receive_pack_update(repo, staged_update)?;
        Ok(None)
    }
}

#[allow(dead_code)]
pub(crate) fn build_staged_receive_pack_update(
    repo: &StoredRepository,
    update: ReceivePackUpdate,
    store: &dyn ObjectStore,
) -> Result<StagedRepoUpdate, ApiError> {
    let live_tree = live_tree(repo);
    let mut staged_changes = Vec::with_capacity(update.changes.len());

    for change in update.changes {
        let old_content = live_tree.get(&change.path).cloned();
        if source_content_matches(old_content.as_ref(), change.content.as_ref()) {
            continue;
        }
        let kind = match (&old_content, &change.content) {
            (None, Some(_)) => StagedFileChangeKind::Added,
            (Some(_), Some(_)) => StagedFileChangeKind::Modified,
            (Some(_), None) => StagedFileChangeKind::Deleted,
            (None, None) => continue,
        };
        let visibility = repo.policy.effective_visibility(&change.path);
        staged_changes.push(StagedFileChange {
            path: change.path,
            line_diff: if repo.settings.review_pushes_before_applying {
                staged_file_line_diff(store, old_content.as_ref(), change.content.as_ref())?
            } else {
                LineDiff::default()
            },
            old_content,
            new_content: change.content,
            visibility,
            kind,
        });
    }

    if staged_changes.is_empty() {
        return Err(ApiError::bad_request(
            "receive-pack update did not change the live tree",
        ));
    }

    Ok(StagedRepoUpdate {
        id: format!("staged_push_{}", repo.graph.commits.len() + 1),
        branch: update.branch,
        base_live_commit_id: repo.graph.commits.last().map(|commit| commit.id.clone()),
        author_id: update.author_id,
        message: update.message,
        git_snapshot: update.git_snapshot,
        changes: staged_changes,
    })
}

pub(super) fn source_content_matches(
    left: Option<&SourceBlob>,
    right: Option<&SourceBlob>,
) -> bool {
    match (left, right) {
        (Some(left), Some(right)) => {
            left.sha256 == right.sha256
                && left.git_oid == right.git_oid
                && left.size_bytes == right.size_bytes
        }
        (None, None) => true,
        _ => false,
    }
}

pub(crate) fn apply_receive_pack_update(
    repo: &mut StoredRepository,
    staged_update: StagedRepoUpdate,
) -> Result<(), ApiError> {
    validate_staged_update_policy(repo, &staged_update)?;
    let owner_ids = repo_owner_ids(repo);
    let visibility_changes = staged_update
        .changes
        .iter()
        .filter(|change| change.new_content.is_some())
        .filter_map(|change| {
            let old_visibility = repo.policy.effective_visibility(&change.path);
            if old_visibility == Visibility::Public
                && change.visibility == Visibility::Private
                && change.old_content.is_some()
            {
                Some(FileVisibilityChange {
                    path: change.path.clone(),
                    old_visibility,
                    new_visibility: change.visibility,
                    current_content: change.new_content.clone(),
                })
            } else {
                None
            }
        })
        .collect::<Vec<_>>();
    for change in &staged_update.changes {
        if change.new_content.is_none() {
            continue;
        }

        let rule = staged_visibility_rule(change, &owner_ids);
        repo.policy.add_rule(rule).map_err(ApiError::bad_request)?;
    }

    let parent_ids = repo
        .graph
        .commits
        .last()
        .map(|commit| vec![commit.id.clone()])
        .unwrap_or_default();
    repo.graph.commits.push(LogicalCommit {
        id: format!("rv_push_{}", repo.graph.commits.len() + 1),
        parent_ids,
        author_id: staged_update.author_id,
        author_visibility: AuthorVisibility::Visible,
        message: staged_update.message,
        mixed_policy: MixedCommitPolicy::SyntheticPublicCommit,
        changes: staged_update
            .changes
            .into_iter()
            .map(|change| FileChange {
                visibility: applied_file_visibility(repo, &change),
                path: change.path,
                old_content: change.old_content,
                new_content: change.new_content,
            })
            .collect(),
        visibility_changes,
    });
    repo.git_snapshot = Some(staged_update.git_snapshot);
    Ok(())
}

fn applied_file_visibility(repo: &StoredRepository, change: &StagedFileChange) -> Visibility {
    if change.new_content.is_none() {
        repo.policy.effective_visibility(&change.path)
    } else {
        change.visibility
    }
}

pub(crate) fn validate_staged_update_policy(
    repo: &StoredRepository,
    staged_update: &StagedRepoUpdate,
) -> Result<(), ApiError> {
    let owner_ids = repo_owner_ids(repo);
    let mut policy = repo.policy.clone();
    for change in &staged_update.changes {
        if change.new_content.is_none() {
            continue;
        }

        let rule = staged_visibility_rule(change, &owner_ids);
        policy.add_rule(rule).map_err(ApiError::bad_request)?;
    }

    Ok(())
}

pub(crate) fn staged_visibility_rule(
    change: &StagedFileChange,
    owner_ids: &[String],
) -> VisibilityRule {
    match change.visibility {
        Visibility::Public => VisibilityRule::public(change.path.clone()),
        Visibility::Private => VisibilityRule::private(change.path.clone(), owner_ids.to_vec()),
    }
}
