use super::{
    policy::{Policy, PolicyError, ScopePath, Visibility, VisibilityRule},
    projection::{AuthorVisibility, FileChange, LogicalCommit, VisibilityEvent},
    repo_config::{HistoryRewriteAction, HistoryRewriteRequest, RepoConfig},
    store::{GitHead, GitSegment, RepoPublicationState, SourceBlob, StoredRepository},
};
use std::collections::{BTreeMap, BTreeSet};

pub type ReviewedUpdateResult<T> = Result<T, ReviewedUpdateError>;

#[derive(Debug)]
pub enum ReviewedUpdateError {
    BadRequest(&'static str),
    Conflict(&'static str),
    InvalidPolicy(PolicyError),
}

#[derive(Clone, Debug)]
pub struct ReviewedContentChange {
    pub path: ScopePath,
    pub content: Option<SourceBlob>,
}

#[derive(Clone, Debug)]
pub struct ReviewedUpdateInput {
    pub branch: String,
    pub author_id: String,
    pub message: String,
    pub git_head: GitHead,
    pub git_segment: GitSegment,
    pub changes: Vec<ReviewedContentChange>,
    pub previous_config: Option<RepoConfig>,
    pub config: RepoConfig,
}

#[derive(Clone, Debug)]
pub struct ContentPushState {
    pub change_version: u64,
    pub policy: Policy,
    pub repo_config: RepoConfig,
    pub previous_commit_id: Option<String>,
    pub live_files: BTreeMap<ScopePath, SourceBlob>,
}

#[derive(Clone, Debug)]
pub struct AcceptedContentPush {
    pub change_version: u64,
    pub policy: Policy,
    pub git_head: GitHead,
    pub git_segment: GitSegment,
    pub logical_commit: LogicalCommit,
}

#[derive(Clone, Debug)]
pub struct ReviewedConfigUpdateInput {
    pub author_id: String,
    pub config: RepoConfig,
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

pub fn apply_reviewed_update_to_repo(
    repo: &mut StoredRepository,
    update: ReviewedUpdateInput,
) -> ReviewedUpdateResult<()> {
    if update.changes.is_empty() {
        return Err(ReviewedUpdateError::BadRequest(
            "update must include file changes",
        ));
    }
    if update.config == repo.repo_config
        && update
            .previous_config
            .as_ref()
            .is_some_and(|previous| previous == &repo.repo_config)
    {
        return apply_content_only_update(repo, update);
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
        return Err(ReviewedUpdateError::BadRequest(
            "update did not change the live tree",
        ));
    }

    let changed_paths = file_changes
        .iter()
        .map(|change| change.path.clone())
        .collect::<BTreeSet<_>>();
    let logical_id = format!("rv_push_{}", update.git_head.head_oid);
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

    let next_policy = policy_from_config_for_tree(&update.config, new_tree.keys())?;
    let next_default_visibility = update.config.visibility.default_visibility().into();
    let next_config = update.config.clone();

    let parent_ids = after_commit_id.into_iter().collect::<Vec<_>>();
    repo.graph.commits.push(LogicalCommit {
        id: logical_id,
        parent_ids,
        author_id: update.author_id,
        author_visibility: AuthorVisibility::Visible,
        message: update.message,
        changes: file_changes,
    });
    repo.live_files = new_tree;
    repo.policy = next_policy;
    repo.record.default_visibility = next_default_visibility;
    repo.repo_config = next_config;
    repo.visibility_events.extend(visibility_events);
    repo.git_segments.push(update.git_segment);
    repo.git_head = Some(update.git_head);
    repo.first_push_token = None;
    repo.record.publication_state = RepoPublicationState::Published;
    repo.bump_change_version();
    Ok(())
}

fn apply_content_only_update(
    repo: &mut StoredRepository,
    update: ReviewedUpdateInput,
) -> ReviewedUpdateResult<()> {
    let live_files = update
        .changes
        .iter()
        .filter_map(|change| {
            repo.live_files
                .get(&change.path)
                .cloned()
                .map(|content| (change.path.clone(), content))
        })
        .collect();
    let accepted = accept_content_push(
        ContentPushState {
            change_version: repo.record.change_version,
            policy: repo.policy.clone(),
            repo_config: repo.repo_config.clone(),
            previous_commit_id: repo.graph.commits.last().map(|commit| commit.id.clone()),
            live_files,
        },
        update,
    )?;
    for change in &accepted.logical_commit.changes {
        match &change.new_content {
            Some(content) => {
                repo.live_files.insert(change.path.clone(), content.clone());
            }
            None => {
                repo.live_files.remove(&change.path);
            }
        }
    }
    repo.record.change_version = accepted.change_version;
    repo.policy = accepted.policy;
    repo.graph.commits.push(accepted.logical_commit);
    repo.git_segments.push(accepted.git_segment);
    repo.git_head = Some(accepted.git_head);
    repo.first_push_token = None;
    repo.record.publication_state = RepoPublicationState::Published;
    Ok(())
}

pub fn accept_content_push(
    state: ContentPushState,
    update: ReviewedUpdateInput,
) -> ReviewedUpdateResult<AcceptedContentPush> {
    accept_content_update(state, update, false)
}

pub fn accept_request_merge(
    state: ContentPushState,
    update: ReviewedUpdateInput,
) -> ReviewedUpdateResult<AcceptedContentPush> {
    accept_content_update(state, update, true)
}

fn accept_content_update(
    state: ContentPushState,
    mut update: ReviewedUpdateInput,
    allow_unchanged_tree: bool,
) -> ReviewedUpdateResult<AcceptedContentPush> {
    if update.changes.is_empty() && !allow_unchanged_tree {
        return Err(ReviewedUpdateError::BadRequest(
            "update must include file changes",
        ));
    }
    if update.config != state.repo_config {
        return Err(ReviewedUpdateError::Conflict(
            "repo config changed since review; rerun scope push",
        ));
    }

    let mut file_changes = Vec::with_capacity(update.changes.len());
    for change in update.changes {
        let old_content = state.live_files.get(&change.path).cloned();
        if source_content_matches(old_content.as_ref(), change.content.as_ref()) {
            continue;
        }
        let visibility = if old_content.is_some() || change.content.is_none() {
            state.policy.effective_visibility(&change.path)
        } else {
            update.config.visibility_for_path(&change.path)
        };
        file_changes.push(FileChange {
            visibility,
            path: change.path,
            old_content,
            new_content: change.content,
        });
    }
    if file_changes.is_empty() && !allow_unchanged_tree {
        return Err(ReviewedUpdateError::BadRequest(
            "update did not change the live tree",
        ));
    }
    let mut policy = state.policy;
    for change in &file_changes {
        match (&change.old_content, &change.new_content) {
            (None, Some(_)) => {
                let rule = match update.config.visibility_for_path(&change.path) {
                    Visibility::Public => VisibilityRule::public(change.path.clone()),
                    Visibility::Private => VisibilityRule::private(change.path.clone()),
                };
                policy
                    .add_rule(rule)
                    .map_err(ReviewedUpdateError::InvalidPolicy)?;
            }
            (Some(_), None) => policy.remove_rule(&change.path),
            _ => {}
        }
    }
    let change_version = state.change_version.saturating_add(1);
    update.git_head.change_version = change_version;
    let logical_prefix = if allow_unchanged_tree {
        "rv_merge"
    } else {
        "rv_push"
    };
    let logical_id = format!("{logical_prefix}_{}", update.git_head.head_oid);
    let logical_commit = LogicalCommit {
        id: logical_id,
        parent_ids: state.previous_commit_id.into_iter().collect(),
        author_id: update.author_id,
        author_visibility: AuthorVisibility::Visible,
        message: update.message,
        changes: file_changes,
    };
    Ok(AcceptedContentPush {
        change_version,
        policy,
        git_head: update.git_head,
        git_segment: update.git_segment,
        logical_commit,
    })
}

pub fn apply_reviewed_config_to_repo(
    repo: &mut StoredRepository,
    update: ReviewedConfigUpdateInput,
) -> ReviewedUpdateResult<bool> {
    if repo.repo_config == update.config {
        return Ok(false);
    }
    let live_tree = repo.live_tree();
    let after_commit_id = repo.graph.commits.last().map(|commit| commit.id.clone());
    let mut next_visibility_event_id = repo.visibility_events.len() + 1;
    let history_rewrites = update
        .config
        .history_rewrites_added_since(Some(&repo.repo_config));
    let history_rewrite = apply_history_rewrites(
        repo,
        &mut next_visibility_event_id,
        HistoryRewriteInput {
            config: &update.config,
            rewrites: &history_rewrites,
            live_tree: &live_tree,
            changed_paths: &BTreeSet::new(),
            after_commit_id: after_commit_id.clone(),
            author_id: &update.author_id,
        },
    );

    let mut visibility_events = history_rewrite.visibility_events;
    for (path, current_content) in &live_tree {
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

        let id = format!("vis_{next_visibility_event_id}");
        next_visibility_event_id += 1;
        visibility_events.push(VisibilityEvent {
            id,
            after_commit_id: after_commit_id.clone(),
            source_commit_id: None,
            author_id: update.author_id.clone(),
            path: path.clone(),
            old_visibility,
            new_visibility,
            current_content: Some(current_content.clone()),
        });
    }

    repo.policy = policy_from_config_for_tree(&update.config, live_tree.keys())?;
    repo.record.default_visibility = update.config.visibility.default_visibility().into();
    repo.repo_config = update.config;
    repo.visibility_events.extend(visibility_events);
    repo.bump_change_version();
    Ok(true)
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

fn policy_from_config_for_tree<'a>(
    config: &RepoConfig,
    paths: impl IntoIterator<Item = &'a ScopePath>,
) -> ReviewedUpdateResult<Policy> {
    let mut policy = Policy::new(config.visibility.default_visibility().into());
    for path in paths {
        let rule = match config.visibility_for_path(path) {
            Visibility::Public => VisibilityRule::public(path.clone()),
            Visibility::Private => VisibilityRule::private(path.clone()),
        };
        policy
            .add_rule(rule)
            .map_err(ReviewedUpdateError::InvalidPolicy)?;
    }
    Ok(policy)
}
