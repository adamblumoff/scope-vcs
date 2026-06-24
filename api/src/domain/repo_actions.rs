use super::{
    policy::{ScopePath, Visibility, VisibilityRule},
    projection::{
        AuthorVisibility, FileChange, FileVisibilityChange, LogicalCommit, MixedCommitPolicy,
    },
    store::{
        CatalogError, FirstPushToken, GitPushToken, PendingImport, RepoPublicationState,
        RepoRecord, RepoRole, RepoSettings, RepoStorageCleanup, SourceBlob, StagedRepoUpdate,
        StoredRepository, UserAccount, pending_import_scope_path,
    },
};
use crate::error::ApiError;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum RepoEffect {
    DeleteRepoStorage(RepoStorageCleanup),
    DeleteSourceBlobs(Vec<SourceBlob>),
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct RepoEffects {
    effects: Vec<RepoEffect>,
}

impl RepoEffects {
    pub(crate) fn is_empty(&self) -> bool {
        self.effects.is_empty()
    }

    pub(crate) fn iter(&self) -> impl Iterator<Item = &RepoEffect> {
        self.effects.iter()
    }

    fn delete_repo_storage(&mut self, cleanup: RepoStorageCleanup) {
        self.effects.push(RepoEffect::DeleteRepoStorage(cleanup));
    }

    fn delete_source_blobs(&mut self, blobs: impl IntoIterator<Item = SourceBlob>) {
        let blobs = blobs.into_iter().collect::<Vec<_>>();
        if !blobs.is_empty() {
            self.effects.push(RepoEffect::DeleteSourceBlobs(blobs));
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct RepoMutation<T> {
    pub(crate) result: T,
    pub(crate) effects: RepoEffects,
}

impl<T> RepoMutation<T> {
    fn new(result: T) -> Self {
        Self {
            result,
            effects: RepoEffects::default(),
        }
    }

    fn with_effects(result: T, effects: RepoEffects) -> Self {
        Self { result, effects }
    }
}

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

pub(crate) fn create_repo(
    owner: &UserAccount,
    name: &str,
    default_visibility: Visibility,
    first_push_token: FirstPushToken,
    git_push_token: GitPushToken,
) -> Result<RepoMutation<StoredRepository>, ApiError> {
    let mut repo = StoredRepository::new(owner, name, default_visibility).map_err(catalog_error)?;
    repo.first_push_token = Some(secretless_first_push_token(first_push_token));
    repo.git_push_token = Some(git_push_token);
    Ok(RepoMutation::new(repo))
}

pub(crate) fn regenerate_setup_tokens(
    repo: &mut StoredRepository,
    user_id: &str,
    first_push_token: FirstPushToken,
    git_push_token: GitPushToken,
) -> Result<RepoMutation<()>, ApiError> {
    ensure_repo_setup_access(repo, user_id)?;
    repo.first_push_token = Some(secretless_first_push_token(first_push_token));
    repo.git_push_token = Some(git_push_token);
    Ok(RepoMutation::new(()))
}

pub(crate) fn set_visibility(
    repo: &mut StoredRepository,
    user_id: &str,
    update_paths: &[ScopePath],
    visibility: Visibility,
) -> Result<RepoMutation<()>, ApiError> {
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
    Ok(RepoMutation::new(()))
}

pub(crate) fn set_staged_visibility(
    repo: &mut StoredRepository,
    user_id: &str,
    update_paths: &[ScopePath],
    visibility: Visibility,
) -> Result<RepoMutation<StagedRepoUpdate>, ApiError> {
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
    Ok(RepoMutation::new(staged_update))
}

pub(crate) fn apply_staged_update(
    repo: &mut StoredRepository,
    user_id: &str,
) -> Result<RepoMutation<StagedRepoUpdate>, ApiError> {
    ensure_repo_owner(repo, user_id)?;
    let old_snapshot = repo.git_snapshot.clone();
    let staged_update = repo
        .staged_update
        .take()
        .ok_or_else(|| ApiError::not_found("no staged update pending"))?;
    let applied = staged_update.clone();
    crate::git::import::apply_receive_pack_update(repo, staged_update)?;
    let mut effects = RepoEffects::default();
    effects.delete_source_blobs(old_snapshot);
    Ok(RepoMutation::with_effects(applied, effects))
}

pub(crate) fn reject_staged_update(
    repo: &mut StoredRepository,
    user_id: &str,
) -> Result<RepoMutation<StagedRepoUpdate>, ApiError> {
    ensure_repo_owner(repo, user_id)?;
    let rejected = repo
        .staged_update
        .take()
        .ok_or_else(|| ApiError::not_found("no staged update pending"))?;
    let mut effects = RepoEffects::default();
    effects.delete_source_blobs(staged_update_blobs(&rejected));
    Ok(RepoMutation::with_effects(rejected, effects))
}

fn staged_update_blobs(staged_update: &StagedRepoUpdate) -> Vec<SourceBlob> {
    std::iter::once(staged_update.git_snapshot.clone())
        .chain(
            staged_update
                .changes
                .iter()
                .filter_map(|change| change.new_content.clone()),
        )
        .collect()
}

pub(crate) fn update_settings(
    repo: &mut StoredRepository,
    user_id: &str,
    settings: RepoSettings,
    default_visibility: Visibility,
) -> Result<RepoMutation<()>, ApiError> {
    ensure_repo_owner(repo, user_id)?;
    repo.settings = settings;
    if repo.record.default_visibility != default_visibility {
        preserve_existing_visibility_for_new_default(repo, default_visibility)?;
    }
    repo.record.default_visibility = default_visibility;
    Ok(RepoMutation::new(()))
}

pub(crate) fn publish_import(
    repo: &mut StoredRepository,
    user_id: &str,
) -> Result<RepoMutation<RepoRecord>, ApiError> {
    ensure_repo_owner(repo, user_id)?;
    preview_publish_import(repo)
}

pub(crate) fn preview_publish_import(
    repo: &mut StoredRepository,
) -> Result<RepoMutation<RepoRecord>, ApiError> {
    ensure_pending_publish(repo)?;
    let pending = repo
        .pending_import
        .take()
        .ok_or_else(|| ApiError::bad_request("repo has no pending import to publish"))?;
    let changes = pending_import_changes(&repo.policy, &pending);
    let parent_ids = repo
        .graph
        .commits
        .last()
        .map(|commit| vec![commit.id.clone()])
        .unwrap_or_default();
    let logical_id = format!(
        "rv_git_{}",
        pending
            .head_oid
            .get(..12)
            .unwrap_or(pending.head_oid.as_str())
    );
    repo.graph.commits.push(LogicalCommit {
        id: logical_id,
        parent_ids,
        author_id: repo.record.owner_user_id.clone(),
        author_visibility: AuthorVisibility::Visible,
        message: format!("Import pushed {}", pending.default_branch),
        mixed_policy: MixedCommitPolicy::SyntheticPublicCommit,
        changes,
        visibility_changes: Vec::new(),
    });
    repo.git_snapshot = Some(pending.git_snapshot);
    repo.record.publication_state = RepoPublicationState::Published;
    repo.first_push_token = None;
    Ok(RepoMutation::new(repo.record.clone()))
}

pub(crate) fn ensure_pending_publish(repo: &StoredRepository) -> Result<(), ApiError> {
    if repo.record.publication_state != RepoPublicationState::PendingPublish {
        return Err(ApiError::bad_request("repo is not pending publish"));
    }
    if repo.pending_import.is_none() {
        return Err(ApiError::bad_request(
            "repo has no pending import to publish",
        ));
    }
    Ok(())
}

pub(crate) fn delete_repo(
    repo: &StoredRepository,
    user_id: &str,
    owner: &str,
    name: &str,
) -> Result<RepoMutation<String>, ApiError> {
    ensure_repo_delete_owner(repo, user_id, owner, name)?;
    let mut effects = RepoEffects::default();
    effects.delete_repo_storage(RepoStorageCleanup {
        owner_handle: owner.to_string(),
        repo_name: name.to_string(),
    });
    effects.delete_source_blobs(repo.source_blobs());
    Ok(RepoMutation::with_effects(repo.record.id.clone(), effects))
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

fn pending_import_changes(
    policy: &super::policy::Policy,
    pending: &PendingImport,
) -> Vec<FileChange> {
    pending
        .files
        .iter()
        .map(|file| {
            let path = pending_import_scope_path(&file.path)
                .expect("pending import paths were validated before persistence");
            FileChange {
                visibility: policy.effective_visibility(&path),
                path,
                old_content: None,
                new_content: Some(file.blob.clone()),
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::{
        policy::{ScopePath, Visibility},
        store::{
            AccountAccess, LineDiff, StagedFileChange, StagedFileChangeKind, StagedRepoUpdate,
            UserAccount,
        },
    };

    #[test]
    fn rejecting_staged_update_returns_source_blob_cleanup_effects() {
        let owner = test_owner();
        let mut repo = StoredRepository::new(&owner, "repo", Visibility::Private).unwrap();
        let staged_snapshot = source_blob("staged-snapshot");
        let staged_blob = source_blob("staged-file");
        repo.staged_update = Some(StagedRepoUpdate {
            id: "staged".to_string(),
            branch: "main".to_string(),
            base_live_commit_id: None,
            author_id: owner.id.clone(),
            message: "Push".to_string(),
            git_snapshot: staged_snapshot.clone(),
            changes: vec![StagedFileChange {
                path: ScopePath::parse("/src/lib.rs").unwrap(),
                old_content: None,
                new_content: Some(staged_blob.clone()),
                line_diff: LineDiff {
                    additions: staged_blob.line_count,
                    deletions: 0,
                },
                visibility: Visibility::Private,
                kind: StagedFileChangeKind::Added,
            }],
        });

        let mutation = reject_staged_update(&mut repo, &owner.id).unwrap();

        assert!(repo.staged_update.is_none());
        assert_eq!(mutation.result.id, "staged");
        assert_eq!(
            source_blob_effect_keys(&mutation.effects),
            vec![staged_snapshot.object_key, staged_blob.object_key]
        );
    }

    #[test]
    fn deleting_repo_returns_storage_and_source_blob_cleanup_effects() {
        let owner = test_owner();
        let mut repo = StoredRepository::new(&owner, "repo", Visibility::Private).unwrap();
        let snapshot = source_blob("live-snapshot");
        repo.git_snapshot = Some(snapshot.clone());

        let mutation = delete_repo(&repo, &owner.id, &owner.handle, &repo.record.name).unwrap();

        assert_eq!(mutation.result, repo.record.id);
        assert_eq!(
            repo_storage_effects(&mutation.effects),
            vec![RepoStorageCleanup {
                owner_handle: owner.handle,
                repo_name: "repo".to_string(),
            }]
        );
        assert_eq!(
            source_blob_effect_keys(&mutation.effects),
            vec![snapshot.object_key]
        );
    }

    fn repo_storage_effects(effects: &RepoEffects) -> Vec<RepoStorageCleanup> {
        effects
            .iter()
            .filter_map(|effect| match effect {
                RepoEffect::DeleteRepoStorage(cleanup) => Some(cleanup.clone()),
                RepoEffect::DeleteSourceBlobs(_) => None,
            })
            .collect()
    }

    fn source_blob_effect_keys(effects: &RepoEffects) -> Vec<String> {
        effects
            .iter()
            .flat_map(|effect| match effect {
                RepoEffect::DeleteRepoStorage(_) => Vec::new(),
                RepoEffect::DeleteSourceBlobs(blobs) => blobs
                    .iter()
                    .map(|blob| blob.object_key.clone())
                    .collect::<Vec<_>>(),
            })
            .collect()
    }

    fn test_owner() -> UserAccount {
        UserAccount {
            id: "owner-id".to_string(),
            handle: "owner".to_string(),
            email: "owner@example.com".to_string(),
            email_verified: true,
            access: AccountAccess::Member,
        }
    }

    fn source_blob(label: &str) -> SourceBlob {
        SourceBlob {
            object_key: format!("objects/{label}"),
            sha256: format!("sha256-{label}"),
            git_oid: format!("oid-{label}"),
            size_bytes: label.len() as u64,
            line_count: label.lines().count(),
        }
    }
}
