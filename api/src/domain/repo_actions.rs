use super::{
    policy::{ScopePath, Visibility, VisibilityRule},
    projection::{AuthorVisibility, FileVisibilityChange, LogicalCommit, MixedCommitPolicy},
    store::{
        CatalogError, FirstPushToken, RepoPublicationState, RepoRole, RepoSettings, SourceBlob,
        StagedRepoUpdate, StoredRepository, pending_import_scope_path,
    },
};
use crate::error::ApiError;

pub(crate) fn ensure_repo_owner(repo: &StoredRepository, user_id: &str) -> Result<(), ApiError> {
    let role = repo
        .memberships
        .iter()
        .find(|membership| membership.user_id == user_id)
        .map(|membership| membership.role);
    if role != Some(RepoRole::Owner) {
        return Err(ApiError::forbidden("owner role required"));
    }
    Ok(())
}

pub(crate) fn ensure_repo_setup_access(
    repo: &StoredRepository,
    user_id: &str,
) -> Result<(), ApiError> {
    let role = repo
        .memberships
        .iter()
        .find(|membership| membership.user_id == user_id)
        .map(|membership| membership.role);
    if role != Some(RepoRole::Owner) {
        return Err(ApiError::not_found(format!(
            "repo {} not found",
            repo.record.id
        )));
    }
    if repo.record.publication_state != RepoPublicationState::PendingFirstPush {
        return Err(ApiError::conflict(
            "setup token is only available before the first push",
        ));
    }
    Ok(())
}

pub(crate) fn ensure_repo_delete_owner(
    repo: &StoredRepository,
    user_id: &str,
    owner: &str,
    name: &str,
) -> Result<(), ApiError> {
    match ensure_repo_owner(repo, user_id) {
        Ok(()) => Ok(()),
        Err(_) => Err(hidden_repo_not_found(owner, name)),
    }
}

pub(crate) fn hidden_repo_not_found(owner: &str, name: &str) -> ApiError {
    ApiError::not_found(format!("repo {owner}/{name} not found"))
}

pub(crate) fn secretless_first_push_token(mut token: FirstPushToken) -> FirstPushToken {
    token.secret = None;
    token
}

pub(crate) fn catalog_error(error: CatalogError) -> ApiError {
    match error {
        CatalogError::InvalidRepositoryName(message) => ApiError::bad_request(message),
        CatalogError::RepositoryExists(repo) => {
            ApiError::conflict(format!("repo {repo} already exists"))
        }
    }
}

pub(crate) fn apply_repo_file_visibility(
    repo: &mut StoredRepository,
    user_id: &str,
    update_paths: &[ScopePath],
    visibility: Visibility,
) -> Result<(), ApiError> {
    if update_paths.is_empty() {
        return Err(ApiError::bad_request("at least one file path is required"));
    }
    ensure_repo_owner(repo, user_id)?;
    if visibility == Visibility::Public {
        for update_path in update_paths {
            if !repo.has_file_for_visibility_update(update_path) {
                return Err(ApiError::bad_request(format!(
                    "file {} must be tracked by Git before it can be made public",
                    update_path.as_str()
                )));
            }
        }
    }

    let owner_ids = repo.owner_ids();
    let record_visibility_history =
        repo.record.publication_state == RepoPublicationState::Published;
    let live_tree = if record_visibility_history {
        repo.live_tree()
    } else {
        Default::default()
    };
    let mut visibility_changes = Vec::new();
    for update_path in update_paths {
        let old_visibility = repo.policy.effective_visibility(update_path);
        if record_visibility_history && old_visibility != visibility {
            visibility_changes.push(FileVisibilityChange {
                path: update_path.clone(),
                old_visibility,
                new_visibility: visibility,
                current_content: live_tree.get(update_path).cloned(),
            });
        }
        let rule = match visibility {
            Visibility::Public => VisibilityRule::public(update_path.clone()),
            Visibility::Private => VisibilityRule::private(update_path.clone(), owner_ids.clone()),
        };
        repo.policy.add_rule(rule).map_err(ApiError::bad_request)?;
    }
    if !visibility_changes.is_empty() {
        let parent_ids = repo
            .graph
            .commits
            .last()
            .map(|commit| vec![commit.id.clone()])
            .unwrap_or_default();
        repo.graph.commits.push(LogicalCommit {
            id: format!("rv_visibility_{}", repo.graph.commits.len() + 1),
            parent_ids,
            author_id: user_id.to_string(),
            author_visibility: AuthorVisibility::Visible,
            message: "Update file visibility".to_string(),
            mixed_policy: MixedCommitPolicy::SyntheticPublicCommit,
            changes: Vec::new(),
            visibility_changes,
        });
    }
    Ok(())
}

pub(crate) fn update_staged_file_visibility_for_repo(
    repo: &mut StoredRepository,
    user_id: &str,
    update_paths: &[ScopePath],
    visibility: Visibility,
) -> Result<StagedRepoUpdate, ApiError> {
    if update_paths.is_empty() {
        return Err(ApiError::bad_request("at least one file path is required"));
    }
    ensure_repo_owner(repo, user_id)?;

    let mut staged_update = repo
        .staged_update
        .clone()
        .ok_or_else(|| ApiError::not_found("no staged update pending"))?;
    for path in update_paths {
        let file = staged_update
            .changes
            .iter_mut()
            .find(|change| change.path == *path)
            .ok_or_else(|| ApiError::not_found(format!("staged file {} not found", path)))?;
        file.visibility = visibility;
    }
    crate::git::import::validate_staged_update_policy(repo, &staged_update)?;
    repo.staged_update = Some(staged_update.clone());
    Ok(staged_update)
}

pub(crate) fn apply_staged_update_for_repo(
    repo: &mut StoredRepository,
    user_id: &str,
) -> Result<StagedRepoUpdate, ApiError> {
    ensure_repo_owner(repo, user_id)?;
    let staged_update = repo
        .staged_update
        .take()
        .ok_or_else(|| ApiError::not_found("no staged update pending"))?;
    let applied = staged_update.clone();
    crate::git::import::apply_receive_pack_update(repo, staged_update)?;
    Ok(applied)
}

pub(crate) fn reject_staged_update_for_repo(
    repo: &mut StoredRepository,
    user_id: &str,
) -> Result<StagedRepoUpdate, ApiError> {
    ensure_repo_owner(repo, user_id)?;
    repo.staged_update
        .take()
        .ok_or_else(|| ApiError::not_found("no staged update pending"))
}

pub(crate) fn rejected_staged_update_blobs(staged_update: &StagedRepoUpdate) -> Vec<SourceBlob> {
    std::iter::once(staged_update.git_snapshot.clone())
        .chain(
            staged_update
                .changes
                .iter()
                .filter_map(|change| change.new_content.clone()),
        )
        .collect()
}

pub(crate) fn apply_repo_settings(
    repo: &mut StoredRepository,
    user_id: &str,
    settings: RepoSettings,
    default_visibility: Visibility,
) -> Result<(), ApiError> {
    ensure_repo_owner(repo, user_id)?;
    repo.settings = settings;
    if repo.record.default_visibility != default_visibility {
        preserve_existing_visibility_for_new_default(repo, default_visibility)?;
    }
    repo.record.default_visibility = default_visibility;
    Ok(())
}

fn preserve_existing_visibility_for_new_default(
    repo: &mut StoredRepository,
    default_visibility: Visibility,
) -> Result<(), ApiError> {
    let existing_visibility = existing_repo_paths(repo)?
        .into_iter()
        .map(|path| {
            let visibility = repo.policy.effective_visibility(&path);
            (path, visibility)
        })
        .collect::<Vec<_>>();
    let owner_ids = repo.owner_ids();

    repo.policy.set_default_visibility(default_visibility);
    for (path, visibility) in existing_visibility {
        if repo.policy.effective_visibility(&path) == visibility {
            continue;
        }

        let rule = match visibility {
            Visibility::Public => VisibilityRule::public(path),
            Visibility::Private => VisibilityRule::private(path, owner_ids.clone()),
        };
        repo.policy.add_rule(rule).map_err(ApiError::bad_request)?;
    }
    Ok(())
}

fn existing_repo_paths(repo: &StoredRepository) -> Result<Vec<ScopePath>, ApiError> {
    if repo.record.publication_state == RepoPublicationState::PendingPublish {
        let Some(pending_import) = repo.pending_import.as_ref() else {
            return Ok(Vec::new());
        };
        return pending_import
            .files
            .iter()
            .map(|file| pending_import_scope_path(&file.path).map_err(ApiError::bad_request))
            .collect();
    }

    Ok(repo.live_tree().into_keys().collect())
}
