mod artifacts;
mod repo_io;
mod staging;

#[cfg(test)]
pub(crate) use self::artifacts::pending_import_from_staging_repo;
pub(crate) use self::artifacts::{
    receive_pack_update_from_staging_repo, reviewed_update_from_staging_repo,
};
pub(crate) use self::repo_io::{git_refs, run_git, safe_repo_key};
#[cfg(test)]
pub(crate) use self::repo_io::{
    git_stdout_text, git_tree_files, run_git_output, validate_pushed_file_path,
};
pub(crate) use self::staging::ReceivePackUpdate;
#[cfg(test)]
pub(crate) use self::staging::{ReceivePackFileChange, stage_receive_pack_update};
use self::staging::{apply_receive_pack_update, receive_pack_update_changes_visibility};
use crate::domain::{
    policy::ScopePath,
    repo_config::{REPO_CONFIG_PATH, RepoConfig},
    store::{RepositoryActor, StoredRepository},
};
use crate::object_store::{ObjectStore, source_blob_bytes};
use crate::{
    db::RepositoryMutation, error::ApiError, git::PersistedReceivePackUpdate, state::AppState,
};

#[cfg(test)]
pub(crate) fn persist_receive_pack_update(
    state: &AppState,
    owner: &str,
    repo_name: &str,
    update: ReceivePackUpdate,
) -> Result<PersistedReceivePackUpdate, ApiError> {
    state
        .metadata
        .mutate_repository(owner, repo_name, move |repo| {
            ensure_receive_pack_base_matches(repo, &update)?;
            apply_receive_pack_update(repo, update)?;
            Ok(RepositoryMutation::new(PersistedReceivePackUpdate::Applied))
        })
}

pub(crate) fn persist_receive_pack_update_and_promote(
    state: &AppState,
    owner: &str,
    repo_name: &str,
    update: ReceivePackUpdate,
    author_id: &str,
) -> Result<PersistedReceivePackUpdate, ApiError> {
    let uploaded_blobs = update.uploaded_blobs.clone();
    let author_id = author_id.to_string();
    let object_store = state.object_store.clone();

    let persisted = state
        .metadata
        .mutate_repository(owner, repo_name, move |repo| {
            let old_snapshot = repo.git_snapshot.clone();
            let mut cleanup_blobs = uploaded_blobs;
            let mut update = update;
            let access = repo.access_for_user_id(&author_id);
            let can_receive_push = access.can_push
                || repo.is_waiting_for_first_push() && repo.is_owner_user(&author_id);
            if !can_receive_push {
                let message = if access.actor == RepositoryActor::Public {
                    "repo membership required"
                } else {
                    "push permission required"
                };
                return Err(ApiError::forbidden(message));
            }
            let previous_config = previous_repo_config(repo, object_store.as_ref())?;
            if !access.can_change_file_visibility
                && receive_pack_update_changes_visibility(repo, previous_config.as_ref(), &update)
            {
                return Err(ApiError::forbidden("file visibility permission required"));
            }
            update.previous_config = previous_config;
            ensure_receive_pack_base_matches(repo, &update)?;
            apply_receive_pack_update(repo, update)?;
            cleanup_blobs.extend(old_snapshot);
            let persisted = PersistedReceivePackUpdate::Applied;
            Ok(RepositoryMutation::with_source_blob_deletions(
                persisted,
                cleanup_blobs,
            ))
        })?;
    crate::state::best_effort_drain_pending_source_blob_deletions(state);
    Ok(persisted)
}

fn previous_repo_config(
    repo: &StoredRepository,
    object_store: &dyn ObjectStore,
) -> Result<Option<RepoConfig>, ApiError> {
    let config_path = ScopePath::parse(REPO_CONFIG_PATH).map_err(ApiError::internal)?;
    let Some(config_blob) = repo.live_tree().get(&config_path).cloned() else {
        return Ok(None);
    };
    let bytes = source_blob_bytes(object_store, &config_blob)?;
    RepoConfig::parse_json(&bytes).map(Some).map_err(|error| {
        ApiError::internal_message(format!("stored repo config is invalid: {error}"))
    })
}

fn ensure_receive_pack_base_matches(
    repo: &StoredRepository,
    update: &ReceivePackUpdate,
) -> Result<(), ApiError> {
    let Some(expected_base_key) = update.base_git_snapshot_key.as_ref() else {
        return Ok(());
    };
    let actual_base_key = repo
        .git_snapshot
        .as_ref()
        .map(|snapshot| snapshot.object_key.as_str());
    if actual_base_key == expected_base_key.as_deref() {
        Ok(())
    } else {
        Err(ApiError::conflict(
            "repo changed since push was reviewed; rerun scope push",
        ))
    }
}
