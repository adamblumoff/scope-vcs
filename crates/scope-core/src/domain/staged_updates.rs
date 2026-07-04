use super::{
    policy::{Policy, PolicyError, ScopePath, Visibility, VisibilityRule},
    projection::{AuthorVisibility, FileChange, LogicalCommit, VisibilityEvent},
    repo_config::{HistoryRewriteAction, HistoryRewriteRequest, RepoConfig},
    store::{
        RepoPublicationState, SourceBlob, StagedFileChange, StagedFileChangeKind, StagedRepoUpdate,
        StoredRepository,
    },
};
use std::collections::{BTreeMap, BTreeSet};

pub type StagedUpdateResult<T> = Result<T, StagedUpdateError>;

#[derive(Debug)]
pub enum StagedUpdateError {
    BadRequest(&'static str),
    Conflict(&'static str),
    InvalidPolicy(PolicyError),
}

#[derive(Clone, Debug)]
pub struct StagedContentChange {
    pub path: ScopePath,
    pub content: Option<SourceBlob>,
}

#[derive(Clone, Debug)]
pub struct StagedUpdateInput {
    pub branch: String,
    pub author_id: String,
    pub message: String,
    pub git_snapshot: SourceBlob,
    pub changes: Vec<StagedContentChange>,
}

#[derive(Clone, Debug)]
pub struct ReviewedUpdateInput {
    pub branch: String,
    pub author_id: String,
    pub message: String,
    pub git_snapshot: SourceBlob,
    pub changes: Vec<StagedContentChange>,
    pub previous_config: Option<RepoConfig>,
    pub config: RepoConfig,
}

pub fn stage_staged_update(
    repo: &mut StoredRepository,
    update: StagedUpdateInput,
    can_apply_changes: bool,
) -> StagedUpdateResult<Option<StagedRepoUpdate>> {
    if repo.record.publication_state != RepoPublicationState::Published {
        return Err(StagedUpdateError::Conflict(
            "repo must be published before push",
        ));
    }
    if update.changes.is_empty() {
        return Err(StagedUpdateError::BadRequest(
            "update must include file changes",
        ));
    }
    if repo.staged_update.is_some() {
        return Err(StagedUpdateError::Conflict(
            "a staged update is already pending",
        ));
    }

    let will_stage = repo.settings.review_pushes_before_applying || !can_apply_changes;
    let staged_update = build_staged_update(repo, update)?;
    if will_stage {
        repo.staged_update = Some(staged_update.clone());
        repo.bump_change_version();
        Ok(Some(staged_update))
    } else {
        apply_staged_update_to_repo(repo, staged_update)?;
        Ok(None)
    }
}

pub fn build_staged_update(
    repo: &StoredRepository,
    update: StagedUpdateInput,
) -> StagedUpdateResult<StagedRepoUpdate> {
    let live_tree = repo.live_tree();
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
            old_content,
            new_content: change.content,
            visibility,
            kind,
        });
    }

    if staged_changes.is_empty() {
        return Err(StagedUpdateError::BadRequest(
            "update did not change the live tree",
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

pub fn source_content_matches(left: Option<&SourceBlob>, right: Option<&SourceBlob>) -> bool {
    match (left, right) {
        (Some(left), Some(right)) => {
            left.sha256 == right.sha256
                && left.git_oid == right.git_oid
                && left.git_file_mode == right.git_file_mode
                && left.size_bytes == right.size_bytes
        }
        (None, None) => true,
        _ => false,
    }
}

pub fn apply_staged_update_to_repo(
    repo: &mut StoredRepository,
    staged_update: StagedRepoUpdate,
) -> StagedUpdateResult<()> {
    validate_staged_update_policy(repo, &staged_update)?;
    let logical_id = format!("rv_push_{}", repo.graph.commits.len() + 1);
    let after_commit_id = repo.graph.commits.last().map(|commit| commit.id.clone());
    let mut next_visibility_event_id = repo.visibility_events.len() + 1;
    let visibility_events = staged_update
        .changes
        .iter()
        .filter(|change| change.new_content.is_some())
        .filter_map(|change| {
            let old_visibility = repo.policy.effective_visibility(&change.path);
            if old_visibility != change.visibility {
                if old_visibility == Visibility::Public
                    && change.visibility == Visibility::Private
                    && change.old_content.is_none()
                {
                    return None;
                }

                let id = format!("vis_{next_visibility_event_id}");
                next_visibility_event_id += 1;
                let boundary_after_commit_id = (old_visibility == Visibility::Public
                    && change.visibility == Visibility::Private)
                    .then(|| after_commit_id.clone())
                    .flatten();
                Some(VisibilityEvent {
                    id,
                    after_commit_id: boundary_after_commit_id,
                    source_commit_id: Some(logical_id.clone()),
                    author_id: staged_update.author_id.clone(),
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

        let rule = staged_visibility_rule(change);
        repo.policy
            .add_rule(rule)
            .map_err(StagedUpdateError::InvalidPolicy)?;
    }

    let parent_ids = repo
        .graph
        .commits
        .last()
        .map(|commit| vec![commit.id.clone()])
        .unwrap_or_default();
    repo.graph.commits.push(LogicalCommit {
        id: logical_id,
        parent_ids,
        author_id: staged_update.author_id,
        author_visibility: AuthorVisibility::Visible,
        message: staged_update.message,
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
    });
    repo.visibility_events.extend(visibility_events);
    repo.git_snapshot = Some(staged_update.git_snapshot);
    repo.bump_change_version();
    Ok(())
}

pub fn apply_reviewed_update_to_repo(
    repo: &mut StoredRepository,
    update: ReviewedUpdateInput,
) -> StagedUpdateResult<()> {
    if update.changes.is_empty() {
        return Err(StagedUpdateError::BadRequest(
            "update must include file changes",
        ));
    }
    if repo.staged_update.is_some() {
        return Err(StagedUpdateError::Conflict(
            "a staged update is already pending",
        ));
    }

    let old_tree = repo.live_tree();
    let mut new_tree = old_tree.clone();
    let mut file_changes = Vec::with_capacity(update.changes.len());
    for change in update.changes {
        let old_content = old_tree.get(&change.path).cloned();
        if source_content_matches(old_content.as_ref(), change.content.as_ref()) {
            continue;
        }

        match &change.content {
            Some(content) => {
                new_tree.insert(change.path.clone(), content.clone());
            }
            None => {
                new_tree.remove(&change.path);
            }
        }

        let visibility = if change.content.is_some() {
            update.config.visibility_for_path(&change.path)
        } else {
            repo.policy.effective_visibility(&change.path)
        };
        file_changes.push(FileChange {
            visibility,
            path: change.path,
            old_content,
            new_content: change.content,
        });
    }

    if file_changes.is_empty() {
        return Err(StagedUpdateError::BadRequest(
            "update did not change the live tree",
        ));
    }

    let changed_paths = file_changes
        .iter()
        .map(|change| change.path.clone())
        .collect::<BTreeSet<_>>();
    let logical_id = format!("rv_push_{}", repo.graph.commits.len() + 1);
    let after_commit_id = repo.graph.commits.last().map(|commit| commit.id.clone());
    let mut next_visibility_event_id = repo.visibility_events.len() + 1;
    let history_rewrites = update
        .config
        .history_rewrites_added_since(update.previous_config.as_ref());
    let history_rewrite = apply_history_rewrites(
        repo,
        &mut next_visibility_event_id,
        HistoryRewriteInput {
            config: &update.config,
            rewrites: &history_rewrites,
            live_tree: &old_tree,
            changed_paths: &changed_paths,
            after_commit_id: after_commit_id.clone(),
            author_id: &update.author_id,
        },
    );
    for change in &mut file_changes {
        if change.new_content.is_none() && history_rewrite.redacted_paths.contains(&change.path) {
            change.visibility = Visibility::Private;
        }
    }
    let mut visibility_events = history_rewrite.visibility_events;
    for (path, current_content) in &new_tree {
        let old_visibility = repo.policy.effective_visibility(path);
        let new_visibility = update.config.visibility_for_path(path);
        if old_visibility == new_visibility {
            continue;
        }
        if history_rewrite.redacted_paths.contains(path)
            && old_visibility == Visibility::Public
            && new_visibility == Visibility::Private
        {
            continue;
        }
        if old_visibility == Visibility::Public
            && new_visibility == Visibility::Private
            && !old_tree.contains_key(path)
        {
            continue;
        }

        let id = format!("vis_{next_visibility_event_id}");
        next_visibility_event_id += 1;
        let boundary_after_commit_id = match (old_visibility, new_visibility) {
            (Visibility::Public, Visibility::Private) => after_commit_id.clone(),
            _ if changed_paths.contains(path) => None,
            _ => after_commit_id.clone(),
        };
        visibility_events.push(VisibilityEvent {
            id,
            after_commit_id: boundary_after_commit_id,
            source_commit_id: changed_paths.contains(path).then(|| logical_id.clone()),
            author_id: update.author_id.clone(),
            path: path.clone(),
            old_visibility,
            new_visibility,
            current_content: Some(current_content.clone()),
        });
    }

    let mut next_policy = Policy::new(update.config.visibility.default_visibility().into());
    for path in new_tree.keys() {
        let rule = match update.config.visibility_for_path(path) {
            Visibility::Public => VisibilityRule::public(path.clone()),
            Visibility::Private => VisibilityRule::private(path.clone()),
        };
        next_policy
            .add_rule(rule)
            .map_err(StagedUpdateError::InvalidPolicy)?;
    }

    let parent_ids = after_commit_id.into_iter().collect::<Vec<_>>();
    repo.graph.commits.push(LogicalCommit {
        id: logical_id,
        parent_ids,
        author_id: update.author_id,
        author_visibility: AuthorVisibility::Visible,
        message: update.message,
        changes: file_changes,
    });
    repo.policy = next_policy;
    repo.record.default_visibility = update.config.visibility.default_visibility().into();
    repo.visibility_events.extend(visibility_events);
    repo.git_snapshot = Some(update.git_snapshot);
    repo.pending_import = None;
    repo.staged_update = None;
    repo.first_push_token = None;
    repo.record.publication_state = RepoPublicationState::Published;
    repo.bump_change_version();
    Ok(())
}

struct HistoryRewriteResult {
    visibility_events: Vec<VisibilityEvent>,
    redacted_paths: BTreeSet<ScopePath>,
}

struct HistoryRewriteInput<'a> {
    config: &'a RepoConfig,
    rewrites: &'a [HistoryRewriteRequest],
    live_tree: &'a BTreeMap<ScopePath, SourceBlob>,
    changed_paths: &'a BTreeSet<ScopePath>,
    after_commit_id: Option<String>,
    author_id: &'a str,
}

fn apply_history_rewrites(
    repo: &mut StoredRepository,
    next_visibility_event_id: &mut usize,
    input: HistoryRewriteInput<'_>,
) -> HistoryRewriteResult {
    let HistoryRewriteInput {
        config,
        rewrites,
        live_tree,
        changed_paths,
        after_commit_id,
        author_id,
    } = input;

    if rewrites.is_empty() {
        return HistoryRewriteResult {
            visibility_events: Vec::new(),
            redacted_paths: BTreeSet::new(),
        };
    }

    let should_redact = |path: &ScopePath| {
        rewrites.iter().any(|rewrite| {
            rewrite.action == HistoryRewriteAction::RedactPublicHistory
                && rewrite.matches_path(path)
        })
    };

    let mut redacted_paths = BTreeSet::new();
    for commit in &mut repo.graph.commits {
        for change in &mut commit.changes {
            if change.visibility == Visibility::Public && should_redact(&change.path) {
                change.visibility = Visibility::Private;
                redacted_paths.insert(change.path.clone());
            }
        }
    }

    repo.visibility_events.retain(|event| {
        let redact = should_redact(&event.path);
        if redact {
            redacted_paths.insert(event.path.clone());
        }
        !redact
    });

    let mut baseline_events = Vec::new();
    for path in redacted_paths.iter() {
        if changed_paths.contains(path) || config.visibility_for_path(path) != Visibility::Public {
            continue;
        }
        let Some(current_content) = live_tree.get(path) else {
            continue;
        };

        let id = format!("vis_{}", *next_visibility_event_id);
        *next_visibility_event_id += 1;
        baseline_events.push(VisibilityEvent {
            id,
            after_commit_id: after_commit_id.clone(),
            source_commit_id: None,
            author_id: author_id.to_string(),
            path: path.clone(),
            old_visibility: Visibility::Private,
            new_visibility: Visibility::Public,
            current_content: Some(current_content.clone()),
        });
    }

    HistoryRewriteResult {
        visibility_events: baseline_events,
        redacted_paths,
    }
}

fn applied_file_visibility(repo: &StoredRepository, change: &StagedFileChange) -> Visibility {
    if change.new_content.is_none() {
        repo.policy.effective_visibility(&change.path)
    } else {
        change.visibility
    }
}

pub fn validate_staged_update_policy(
    repo: &StoredRepository,
    staged_update: &StagedRepoUpdate,
) -> StagedUpdateResult<()> {
    let mut policy = repo.policy.clone();
    for change in &staged_update.changes {
        if change.new_content.is_none() {
            continue;
        }

        let rule = staged_visibility_rule(change);
        policy
            .add_rule(rule)
            .map_err(StagedUpdateError::InvalidPolicy)?;
    }

    Ok(())
}

pub fn staged_visibility_rule(change: &StagedFileChange) -> VisibilityRule {
    match change.visibility {
        Visibility::Public => VisibilityRule::public(change.path.clone()),
        Visibility::Private => VisibilityRule::private(change.path.clone()),
    }
}
