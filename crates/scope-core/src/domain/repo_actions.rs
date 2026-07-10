use super::{
    policy::{ScopePath, Visibility, VisibilityRule},
    projection::VisibilityEvent,
    repo_config::repo_config_from_policy,
    reviewed_updates::ReviewedUpdateError,
    store::{
        CatalogError, FirstPushToken, GitPushToken, RepoPublicationState, RepoStorageCleanup,
        SourceBlob, StoredRepository, UserAccount,
    },
};
use crate::error::ApiError;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RepoEffect {
    DeleteRepoStorage(RepoStorageCleanup),
    DeleteSourceBlobs(Vec<SourceBlob>),
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RepoEffects {
    effects: Vec<RepoEffect>,
}

impl RepoEffects {
    pub fn is_empty(&self) -> bool {
        self.effects.is_empty()
    }

    pub fn iter(&self) -> impl Iterator<Item = &RepoEffect> {
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
pub struct RepoMutation<T> {
    pub result: T,
    pub effects: RepoEffects,
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

pub fn ensure_repo_owner(repo: &StoredRepository, user_id: &str) -> Result<(), ApiError> {
    if !repo.is_owner_user(user_id) {
        return Err(ApiError::forbidden("owner role required"));
    }
    Ok(())
}

pub fn ensure_repo_member(repo: &StoredRepository, user_id: &str) -> Result<(), ApiError> {
    if repo.is_owner_user(user_id) || repo.member_for_user(user_id).is_some() {
        Ok(())
    } else {
        Err(ApiError::forbidden("repo membership required"))
    }
}

pub fn ensure_can_change_file_visibility(
    repo: &StoredRepository,
    user_id: &str,
) -> Result<(), ApiError> {
    if repo.access_for_user_id(user_id).can_change_file_visibility {
        Ok(())
    } else {
        Err(ApiError::forbidden("file visibility permission required"))
    }
}

pub fn ensure_repo_delete_owner(
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

pub fn hidden_repo_not_found(owner: &str, name: &str) -> ApiError {
    ApiError::not_found(format!("repo {owner}/{name} not found"))
}

pub fn secretless_first_push_token(mut token: FirstPushToken) -> FirstPushToken {
    token.secret = None;
    token
}

pub fn catalog_error(error: CatalogError) -> ApiError {
    match error {
        CatalogError::InvalidRepositoryName(message) => ApiError::bad_request(message),
        CatalogError::RepositoryExists(repo) => {
            ApiError::conflict(format!("repo {repo} already exists"))
        }
    }
}

pub fn reviewed_update_api_error(error: ReviewedUpdateError) -> ApiError {
    match error {
        ReviewedUpdateError::BadRequest(message) => ApiError::bad_request(message),
        ReviewedUpdateError::Conflict(message) => ApiError::conflict(message),
        ReviewedUpdateError::InvalidPolicy(error) => ApiError::bad_request(error),
    }
}

pub fn create_repo(
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

pub fn set_visibility(
    repo: &mut StoredRepository,
    user_id: &str,
    update_paths: &[ScopePath],
    visibility: Visibility,
) -> Result<RepoMutation<()>, ApiError> {
    if update_paths.is_empty() {
        return Err(ApiError::bad_request("at least one file path is required"));
    }
    ensure_can_change_file_visibility(repo, user_id)?;
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

    let record_visibility_history =
        repo.record.publication_state == RepoPublicationState::Published;
    let live_tree = if record_visibility_history {
        repo.live_tree()
    } else {
        Default::default()
    };
    let after_commit_id = repo.graph.commits.last().map(|commit| commit.id.clone());
    let mut visibility_events = Vec::new();
    for update_path in update_paths {
        let old_visibility = repo.policy.effective_visibility(update_path);
        if record_visibility_history && old_visibility != visibility {
            visibility_events.push(VisibilityEvent {
                id: format!(
                    "vis_{}",
                    repo.visibility_events.len() + visibility_events.len() + 1
                ),
                after_commit_id: after_commit_id.clone(),
                source_commit_id: None,
                author_id: user_id.to_string(),
                path: update_path.clone(),
                old_visibility,
                new_visibility: visibility,
                current_content: live_tree.get(update_path).cloned(),
            });
        }
        let rule = match visibility {
            Visibility::Public => VisibilityRule::public(update_path.clone()),
            Visibility::Private => VisibilityRule::private(update_path.clone()),
        };
        repo.policy.add_rule(rule).map_err(ApiError::bad_request)?;
    }
    repo.visibility_events.extend(visibility_events);
    repo.repo_config = repo_config_from_policy(
        &repo.policy,
        repo.record.default_visibility,
        repo.repo_config.history.clone(),
    )
    .map_err(ApiError::bad_request)?;
    repo.bump_change_version();
    Ok(RepoMutation::new(()))
}

pub fn delete_repo(
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::{
        policy::{ScopePath, Visibility},
        store::{DEFAULT_GIT_FILE_MODE, UserAccount},
    };

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

    #[test]
    fn direct_visibility_update_mirrors_repo_config() {
        let owner = test_owner();
        let mut repo = StoredRepository::new(&owner, "repo", Visibility::Public).unwrap();
        let path = ScopePath::parse("/README.md").unwrap();

        set_visibility(
            &mut repo,
            &owner.id,
            std::slice::from_ref(&path),
            Visibility::Private,
        )
        .unwrap();

        assert_eq!(repo.policy.effective_visibility(&path), Visibility::Private);
        assert_eq!(
            repo.repo_config.visibility_for_path(&path),
            Visibility::Private
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
        }
    }

    fn source_blob(label: &str) -> SourceBlob {
        SourceBlob {
            object_key: format!("objects/{label}"),
            sha256: format!("sha256-{label}"),
            git_oid: format!("oid-{label}"),
            git_file_mode: DEFAULT_GIT_FILE_MODE.to_string(),
            size_bytes: label.len() as u64,
        }
    }
}
