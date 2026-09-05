use super::{
    error::{ReviewedUpdateError, ReviewedUpdateResult},
    history_rewrite::{HistoryRewriteInput, apply_history_rewrites},
    policy::policy_from_config_for_tree,
};
use crate::{
    content::SourceBlob,
    error::DomainError,
    policy::{Policy, ScopePath, Visibility, VisibilityRule},
    projection::{FileChange, LogicalCommit, LogicalCommitOrigin},
    repo_config::RepoConfig,
    repo_control::{REPO_RULES_PATH, is_public_request_protected_path},
    repository::{
        RepoLifecycleState, Repository,
        access::{MainPushMode, RepositoryAccess, RepositoryActor},
        git::{GitHead, GitPackSpan},
        updates::RequestMergeOrigin,
    },
    visibility_changes::{VisibilityChange, VisibilityChangeSet, visibility_change_set_id},
};
use std::collections::{BTreeMap, BTreeSet};

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
    pub git_pack_span: GitPackSpan,
    pub changes: Vec<ReviewedContentChange>,
    pub previous_config: Option<RepoConfig>,
    pub config: RepoConfig,
}

#[derive(Clone, Copy, Debug)]
pub struct ReviewedUpdateAuthorization<'a> {
    pub access: RepositoryAccess,
    pub push_mode: MainPushMode,
    pub current_config: &'a RepoConfig,
    pub proposed_config: &'a RepoConfig,
}

pub fn authorize_reviewed_update(
    authorization: ReviewedUpdateAuthorization<'_>,
) -> Result<(), DomainError> {
    if authorization.push_mode == MainPushMode::Denied {
        let message = if authorization.access.actor == RepositoryActor::Public {
            "repo membership required"
        } else {
            "push permission required"
        };
        return Err(DomainError::forbidden(message));
    }
    if !authorization.access.can_change_file_visibility
        && authorization.current_config != authorization.proposed_config
    {
        return Err(DomainError::forbidden(
            "file visibility permission required",
        ));
    }
    Ok(())
}

#[derive(Clone, Debug)]
pub struct ContentPushState {
    pub change_version: u64,
    pub policy: Policy,
    pub repo_config: RepoConfig,
    pub live_files: BTreeMap<ScopePath, SourceBlob>,
    pub git_head: Option<GitHead>,
}

#[derive(Clone, Debug)]
pub struct AcceptedContentPush {
    pub change_version: u64,
    pub policy: Policy,
    pub git_head: GitHead,
    pub git_pack_span: GitPackSpan,
    pub logical_commit: LogicalCommit,
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
    repo: &mut Repository,
    update: ReviewedUpdateInput,
) -> ReviewedUpdateResult<()> {
    validate_git_push_transition(
        repo.git_head.as_ref(),
        &update.git_head,
        &update.git_pack_span,
    )?;
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
    ensure_rules_remain_present(old_tree.contains_key(&repo_rules_path()), &update.changes)?;
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
    let history_rewrites = update
        .config
        .history_rewrites_added_since(update.previous_config.as_ref());
    let history_rewrite = apply_history_rewrites(
        repo,
        HistoryRewriteInput {
            config: &update.config,
            rewrites: &history_rewrites,
            live_tree: &old_tree,
            changed_paths: &changed_paths,
        },
    );
    for change in &mut file_changes {
        if change.new_content.is_none() && history_rewrite.redacted_paths.contains(&change.path) {
            change.visibility = Visibility::Private;
        }
    }
    let mut visibility_changes = history_rewrite.visibility_changes;
    let baseline_paths = visibility_changes
        .iter()
        .map(|change| change.path.clone())
        .collect::<BTreeSet<_>>();
    for (path, current_content) in &new_tree {
        let old_visibility = repo.policy.effective_visibility(path);
        let new_visibility = update.config.visibility_for_path(path);
        if old_visibility == new_visibility {
            continue;
        }
        if baseline_paths.contains(path) {
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

        visibility_changes.push(VisibilityChange {
            path: path.clone(),
            old_visibility,
            new_visibility,
            current_content: Some(current_content.clone()),
        });
    }

    let next_policy = policy_from_config_for_tree(&update.config, new_tree.keys())?;
    let next_config = update.config.clone();

    if !visibility_changes.is_empty() {
        repo.visibility_change_sets.push(
            VisibilityChangeSet::new(
                visibility_change_set_id(repo.record.change_version.saturating_add(1)),
                after_commit_id,
                Some(logical_id.clone()),
                update.author_id.clone(),
                visibility_changes,
            )
            .map_err(ReviewedUpdateError::Conflict)?,
        );
    }

    repo.graph.commits.push(LogicalCommit {
        id: logical_id,
        origin: LogicalCommitOrigin::CanonicalPush {
            source_head_oid: update.git_head.head_oid.clone(),
        },
        author_id: update.author_id,
        message: update.message,
        changes: file_changes,
    });
    repo.live_files = new_tree;
    repo.policy = next_policy;
    repo.repo_config = next_config;
    repo.git_pack_spans.push(update.git_pack_span);
    repo.git_head = Some(update.git_head);
    repo.first_push_token = None;
    repo.record.lifecycle_state = RepoLifecycleState::Ready;
    repo.bump_change_version();
    Ok(())
}

fn apply_content_only_update(
    repo: &mut Repository,
    update: ReviewedUpdateInput,
) -> ReviewedUpdateResult<()> {
    let accepted = accept_content_push(
        ContentPushState {
            change_version: repo.record.change_version,
            policy: repo.policy.clone(),
            repo_config: repo.repo_config.clone(),
            live_files: repo.live_tree(),
            git_head: repo.git_head.clone(),
        },
        update,
    )?;
    apply_accepted_content_push(repo, accepted);
    Ok(())
}

pub fn apply_request_merge_to_repo(
    repo: &mut Repository,
    update: ReviewedUpdateInput,
    origin: RequestMergeOrigin,
) -> ReviewedUpdateResult<()> {
    let accepted = accept_request_merge(
        ContentPushState {
            change_version: repo.record.change_version,
            policy: repo.policy.clone(),
            repo_config: repo.repo_config.clone(),
            live_files: repo.live_tree(),
            git_head: repo.git_head.clone(),
        },
        update,
        origin,
    )?;
    apply_accepted_content_push(repo, accepted);
    Ok(())
}

fn apply_accepted_content_push(repo: &mut Repository, accepted: AcceptedContentPush) {
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
    repo.git_pack_spans.push(accepted.git_pack_span);
    repo.git_head = Some(accepted.git_head);
    repo.first_push_token = None;
    repo.record.lifecycle_state = RepoLifecycleState::Ready;
}

pub fn accept_content_push(
    state: ContentPushState,
    update: ReviewedUpdateInput,
) -> ReviewedUpdateResult<AcceptedContentPush> {
    let source_head_oid = update.git_head.head_oid.clone();
    accept_content_update(
        state,
        update,
        false,
        LogicalCommitOrigin::CanonicalPush { source_head_oid },
    )
}

pub fn accept_request_merge(
    state: ContentPushState,
    update: ReviewedUpdateInput,
    origin: RequestMergeOrigin,
) -> ReviewedUpdateResult<AcceptedContentPush> {
    accept_content_update(state, update, true, origin.into_logical_origin())
}

fn accept_content_update(
    state: ContentPushState,
    mut update: ReviewedUpdateInput,
    allow_unchanged_tree: bool,
    origin: LogicalCommitOrigin,
) -> ReviewedUpdateResult<AcceptedContentPush> {
    validate_git_push_transition(
        state.git_head.as_ref(),
        &update.git_head,
        &update.git_pack_span,
    )?;
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
    ensure_rules_remain_present(
        state.live_files.contains_key(&repo_rules_path()),
        &update.changes,
    )?;

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
    validate_commit_origin(&origin, &file_changes, &update.config)?;
    let mut policy = state.policy;
    for change in &file_changes {
        if change.old_content.is_some() && change.new_content.is_none() {
            policy.remove_rule(&change.path);
        }
    }
    policy
        .add_rules(
            file_changes
                .iter()
                .filter(|change| change.old_content.is_none() && change.new_content.is_some())
                .map(|change| VisibilityRule {
                    path: change.path.clone(),
                    visibility: update.config.visibility_for_path(&change.path),
                }),
        )
        .map_err(ReviewedUpdateError::InvalidPolicy)?;
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
        origin,
        author_id: update.author_id,
        message: update.message,
        changes: file_changes,
    };
    Ok(AcceptedContentPush {
        change_version,
        policy,
        git_head: update.git_head,
        git_pack_span: update.git_pack_span,
        logical_commit,
    })
}

fn validate_git_push_transition(
    previous: Option<&GitHead>,
    next: &GitHead,
    span: &GitPackSpan,
) -> ReviewedUpdateResult<()> {
    if span.first_sequence != next.push_sequence
        || span.last_sequence != next.push_sequence
        || span.geometric_tier != 0
        || span.head_oid != next.head_oid
    {
        return Err(ReviewedUpdateError::Conflict(
            "Git push pack span does not match the logical head",
        ));
    }
    if !next.manifest.git_oid.is_empty() && next.manifest.git_oid != next.head_oid {
        return Err(ReviewedUpdateError::Conflict(
            "Git snapshot manifest does not match the logical head",
        ));
    }
    let expected_sequence = previous
        .map_or(Some(1), |head| head.push_sequence.checked_add(1))
        .ok_or(ReviewedUpdateError::Conflict("Git push sequence overflow"))?;
    let expected_base = previous.map(|head| head.head_oid.as_str());
    if next.push_sequence != expected_sequence || span.base_oid.as_deref() != expected_base {
        return Err(ReviewedUpdateError::Conflict(
            "Git push does not advance the current pack frontier",
        ));
    }
    Ok(())
}

fn repo_rules_path() -> ScopePath {
    ScopePath::parse(REPO_RULES_PATH).expect("canonical repo rules path is valid")
}

fn ensure_rules_remain_present(
    currently_present: bool,
    changes: &[ReviewedContentChange],
) -> ReviewedUpdateResult<()> {
    let resulting_presence = changes
        .iter()
        .rev()
        .find(|change| change.path.as_str() == REPO_RULES_PATH)
        .map_or(currently_present, |change| change.content.is_some());
    if resulting_presence {
        Ok(())
    } else {
        Err(ReviewedUpdateError::BadRequest(
            "repository must contain .scope/RULES.md",
        ))
    }
}

fn validate_commit_origin(
    origin: &LogicalCommitOrigin,
    changes: &[FileChange],
    repo_config: &RepoConfig,
) -> ReviewedUpdateResult<()> {
    let LogicalCommitOrigin::PublicRequestMerge {
        public_base_oid,
        public_parent_oids,
        request_head_oid,
        commits,
        ..
    } = origin
    else {
        return Ok(());
    };

    if changes.iter().any(|change| {
        change.visibility != Visibility::Public || is_public_request_protected_path(&change.path)
    }) {
        return Err(ReviewedUpdateError::Conflict(
            "public request merge contains non-public changes",
        ));
    }
    let Some(last) = commits.last() else {
        return Err(ReviewedUpdateError::Conflict(
            "public request merge has no native commits",
        ));
    };
    if &last.oid != request_head_oid {
        return Err(ReviewedUpdateError::Conflict(
            "public request merge native commits do not end at request head",
        ));
    }
    let range_oids = commits
        .iter()
        .map(|commit| commit.oid.as_str())
        .collect::<BTreeSet<_>>();
    let public_parent_oid_set = public_parent_oids
        .iter()
        .map(String::as_str)
        .collect::<BTreeSet<_>>();
    if range_oids.len() != commits.len()
        || public_parent_oid_set.is_empty()
        || public_parent_oid_set.len() != public_parent_oids.len()
        || public_parent_oids.iter().any(String::is_empty)
        || commits.iter().any(|commit| {
            commit.oid.is_empty()
                || commit.tree_oid.is_empty()
                || commit.parent_oids.is_empty()
                || commit.parent_oids.iter().any(String::is_empty)
        })
    {
        return Err(ReviewedUpdateError::Conflict(
            "public request merge contains malformed native commit facts",
        ));
    }
    let touched_paths = commits
        .iter()
        .flat_map(|commit| commit.changed_paths.iter())
        .collect::<BTreeSet<_>>();
    if touched_paths.iter().any(|path| {
        is_public_request_protected_path(path)
            || repo_config.visibility_for_path(path) != Visibility::Public
    }) || changes
        .iter()
        .any(|change| !touched_paths.contains(&change.path))
    {
        return Err(ReviewedUpdateError::Conflict(
            "public request merge native paths do not cover the logical changes",
        ));
    }
    let mut seen = BTreeSet::new();
    let mut descends_from_public_base = false;
    for commit in commits {
        for parent_oid in &commit.parent_oids {
            descends_from_public_base |= parent_oid == public_base_oid;
            if range_oids.contains(parent_oid.as_str()) {
                if seen.contains(parent_oid.as_str()) {
                    continue;
                }
                return Err(ReviewedUpdateError::Conflict(
                    "public request merge native commits are not ordered ancestor-first",
                ));
            }
            if !public_parent_oid_set.contains(parent_oid.as_str()) {
                return Err(ReviewedUpdateError::Conflict(
                    "public request merge contains a parent outside public history",
                ));
            }
        }
        seen.insert(commit.oid.as_str());
    }
    if !descends_from_public_base || !public_parent_oid_set.contains(public_base_oid.as_str()) {
        return Err(ReviewedUpdateError::Conflict(
            "public request merge does not include the current public base as a parent",
        ));
    }
    Ok(())
}

#[cfg(test)]
mod authorization_tests {
    use super::*;
    use crate::{
        error::DomainErrorKind, repo_config::ConfigVisibility, repository::access::RepositoryActor,
    };

    fn access(
        actor: RepositoryActor,
        can_push: bool,
        can_change_visibility: bool,
    ) -> RepositoryAccess {
        RepositoryAccess {
            actor,
            can_read_private_files: actor != RepositoryActor::Public,
            can_push,
            can_change_file_visibility: can_change_visibility,
            can_apply_changes: false,
            can_manage_members: false,
            can_delete_repo: false,
        }
    }

    #[test]
    fn reviewed_update_authorization_masks_public_push_denial_as_membership_required() {
        let config = RepoConfig::with_default_visibility(ConfigVisibility::Private);
        let error = authorize_reviewed_update(ReviewedUpdateAuthorization {
            access: access(RepositoryActor::Public, false, false),
            push_mode: MainPushMode::Denied,
            current_config: &config,
            proposed_config: &config,
        })
        .expect_err("public actor cannot push main");

        assert_eq!(error.kind, DomainErrorKind::Forbidden);
        assert_eq!(error.message, "repo membership required");
    }

    #[test]
    fn reviewed_update_authorization_rechecks_member_push_permission() {
        let config = RepoConfig::with_default_visibility(ConfigVisibility::Private);
        let error = authorize_reviewed_update(ReviewedUpdateAuthorization {
            access: access(RepositoryActor::Member, false, false),
            push_mode: MainPushMode::Denied,
            current_config: &config,
            proposed_config: &config,
        })
        .expect_err("member without push permission cannot push main");

        assert_eq!(error.kind, DomainErrorKind::Forbidden);
        assert_eq!(error.message, "push permission required");
    }

    #[test]
    fn reviewed_update_authorization_protects_config_changes() {
        let current = RepoConfig::with_default_visibility(ConfigVisibility::Private);
        let proposed = RepoConfig::with_default_visibility(ConfigVisibility::Public);
        let error = authorize_reviewed_update(ReviewedUpdateAuthorization {
            access: access(RepositoryActor::Member, true, false),
            push_mode: MainPushMode::Ready,
            current_config: &current,
            proposed_config: &proposed,
        })
        .expect_err("config change requires visibility permission");

        assert_eq!(error.kind, DomainErrorKind::Forbidden);
        assert_eq!(error.message, "file visibility permission required");
    }

    #[test]
    fn reviewed_update_authorization_allows_content_only_push_without_visibility_permission() {
        let config = RepoConfig::with_default_visibility(ConfigVisibility::Private);
        authorize_reviewed_update(ReviewedUpdateAuthorization {
            access: access(RepositoryActor::Member, true, false),
            push_mode: MainPushMode::Ready,
            current_config: &config,
            proposed_config: &config,
        })
        .expect("content-only push remains allowed");
    }
}
