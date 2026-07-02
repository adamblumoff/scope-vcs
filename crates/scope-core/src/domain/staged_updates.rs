use super::{
    policy::{PolicyError, ScopePath, Visibility, VisibilityRule},
    projection::{AuthorVisibility, FileChange, LogicalCommit, VisibilityEvent},
    store::{
        RepoPublicationState, SourceBlob, StagedFileChange, StagedFileChangeKind, StagedRepoUpdate,
        StoredRepository,
    },
};

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
    let mut next_visibility_event_id = repo.visibility_events.len() + 1;
    let visibility_events = staged_update
        .changes
        .iter()
        .filter(|change| change.new_content.is_some())
        .filter_map(|change| {
            let old_visibility = repo.policy.effective_visibility(&change.path);
            if old_visibility != change.visibility {
                let id = format!("vis_{next_visibility_event_id}");
                next_visibility_event_id += 1;
                Some(VisibilityEvent {
                    id,
                    after_commit_id: None,
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
