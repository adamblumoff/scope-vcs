use crate::{
    content::SourceBlob,
    policy::{ScopePath, Visibility},
    projection::LogicalCommitOrigin,
    repo_config::{HistoryRewriteAction, HistoryRewriteRequest, RepoConfig},
    repository::Repository,
    visibility_changes::VisibilityChange,
};
use std::collections::{BTreeMap, BTreeSet};

pub(super) struct HistoryRewriteResult {
    pub(super) visibility_changes: Vec<VisibilityChange>,
    pub(super) redacted_paths: BTreeSet<ScopePath>,
}

pub(super) struct HistoryRewriteInput<'a> {
    pub(super) config: &'a RepoConfig,
    pub(super) rewrites: &'a [HistoryRewriteRequest],
    pub(super) live_tree: &'a BTreeMap<ScopePath, SourceBlob>,
    pub(super) changed_paths: &'a BTreeSet<ScopePath>,
}

pub(super) fn apply_history_rewrites(
    repo: &mut Repository,
    input: HistoryRewriteInput<'_>,
) -> HistoryRewriteResult {
    let HistoryRewriteInput {
        config,
        rewrites,
        live_tree,
        changed_paths,
    } = input;

    if rewrites.is_empty() {
        return HistoryRewriteResult {
            visibility_changes: Vec::new(),
            redacted_paths: BTreeSet::new(),
        };
    }

    let should_redact = |path: &ScopePath| {
        rewrites.iter().any(|rewrite| {
            rewrite.action == HistoryRewriteAction::RedactPublicHistory
                && rewrite.matches_path(path)
        })
    };

    let commit_indexes = repo
        .graph
        .commits
        .iter()
        .enumerate()
        .map(|(index, commit)| (commit.id.as_str(), index))
        .collect::<BTreeMap<_, _>>();
    let mut invalidate_preservation_from = None;
    for (index, commit) in repo.graph.commits.iter().enumerate() {
        let native_history_matches = match &commit.origin {
            LogicalCommitOrigin::PublicRequestMerge { commits, .. } => commits
                .iter()
                .flat_map(|native| native.changed_paths.iter())
                .any(&should_redact),
            _ => false,
        };
        let logical_history_matches = commit
            .changes
            .iter()
            .any(|change| change.visibility == Visibility::Public && should_redact(&change.path));
        if native_history_matches || logical_history_matches {
            invalidate_preservation_from = Some(
                invalidate_preservation_from.map_or(index, |current: usize| current.min(index)),
            );
        }
    }
    for set in &repo.visibility_change_sets {
        if set.changes.iter().any(|change| should_redact(&change.path)) {
            let index = set
                .anchor_commit_id
                .as_deref()
                .and_then(|commit_id| commit_indexes.get(commit_id).copied())
                .map_or(0, |commit_index| commit_index + 1);
            invalidate_preservation_from = Some(
                invalidate_preservation_from.map_or(index, |current: usize| current.min(index)),
            );
        }
    }

    let mut redacted_paths = BTreeSet::new();
    for (index, commit) in repo.graph.commits.iter_mut().enumerate() {
        if let LogicalCommitOrigin::PublicRequestMerge {
            commits,
            preserve_public_commits,
            ..
        } = &mut commit.origin
        {
            if invalidate_preservation_from.is_some_and(|first| index >= first) {
                *preserve_public_commits = false;
            }
            for path in commits
                .iter()
                .flat_map(|native| native.changed_paths.iter())
                .filter(|path| should_redact(path))
            {
                redacted_paths.insert(path.clone());
            }
        }
        for change in &mut commit.changes {
            if change.visibility == Visibility::Public && should_redact(&change.path) {
                change.visibility = Visibility::Private;
                redacted_paths.insert(change.path.clone());
            }
        }
    }

    for set in &mut repo.visibility_change_sets {
        set.changes.retain(|change| {
            let redact = should_redact(&change.path);
            if redact {
                redacted_paths.insert(change.path.clone());
            }
            !redact
        });
    }
    repo.visibility_change_sets
        .retain(|set| !set.changes.is_empty());

    let mut visibility_changes = Vec::new();
    for path in &redacted_paths {
        if changed_paths.contains(path) || config.visibility_for_path(path) != Visibility::Public {
            continue;
        }
        let Some(current_content) = live_tree.get(path) else {
            continue;
        };

        visibility_changes.push(VisibilityChange {
            path: path.clone(),
            old_visibility: Visibility::Private,
            new_visibility: Visibility::Public,
            current_content: Some(current_content.clone()),
        });
    }

    HistoryRewriteResult {
        visibility_changes,
        redacted_paths,
    }
}
