use super::{
    error::{ReviewedUpdateError, ReviewedUpdateResult},
    history_rewrite::{HistoryRewriteInput, apply_history_rewrites},
    policy::policy_from_config_for_tree,
};
use crate::{
    policy::Visibility,
    repo_config::RepoConfig,
    repository::Repository,
    visibility_changes::{VisibilityChange, VisibilityChangeSet, visibility_change_set_id},
};
use std::collections::BTreeSet;

#[derive(Clone, Debug)]
pub struct ReviewedConfigUpdateInput {
    pub author_id: String,
    pub config: RepoConfig,
}

pub fn apply_reviewed_config_to_repo(
    repo: &mut Repository,
    update: ReviewedConfigUpdateInput,
) -> ReviewedUpdateResult<bool> {
    if repo.repo_config == update.config {
        return Ok(false);
    }
    let live_tree = repo.live_tree();
    let after_commit_id = repo.graph.commits.last().map(|commit| commit.id.clone());
    let history_rewrites = update
        .config
        .history_rewrites_added_since(Some(&repo.repo_config));
    let history_rewrite = apply_history_rewrites(
        repo,
        HistoryRewriteInput {
            config: &update.config,
            rewrites: &history_rewrites,
            live_tree: &live_tree,
            changed_paths: &BTreeSet::new(),
        },
    );

    let mut visibility_changes = history_rewrite.visibility_changes;
    let baseline_paths = visibility_changes
        .iter()
        .map(|change| change.path.clone())
        .collect::<BTreeSet<_>>();
    for (path, current_content) in &live_tree {
        let old_visibility = repo.policy.effective_visibility(path);
        let new_visibility = update.config.visibility_for_path(path);
        if old_visibility == new_visibility || baseline_paths.contains(path) {
            continue;
        }
        if history_rewrite.redacted_paths.contains(path)
            && old_visibility == Visibility::Public
            && new_visibility == Visibility::Private
        {
            continue;
        }

        visibility_changes.push(VisibilityChange {
            path: path.clone(),
            old_visibility,
            new_visibility,
            current_content: Some(current_content.clone()),
        });
    }

    repo.policy = policy_from_config_for_tree(&update.config, live_tree.keys())?;
    repo.repo_config = update.config;
    if !visibility_changes.is_empty() {
        repo.visibility_change_sets.push(
            VisibilityChangeSet::new(
                visibility_change_set_id(repo.record.change_version.saturating_add(1)),
                after_commit_id,
                None,
                update.author_id,
                visibility_changes,
            )
            .map_err(ReviewedUpdateError::Conflict)?,
        );
    }
    repo.bump_change_version();
    Ok(true)
}
