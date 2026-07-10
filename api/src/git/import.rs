mod artifacts;
mod repo_io;
mod staging;

pub(crate) use self::artifacts::{
    receive_pack_update_from_staging_repo, reviewed_update_from_staging_repo,
};
pub(crate) use self::repo_io::{
    git_refs, git_snapshot_from_ref, run_git, run_git_output, safe_repo_key, validate_pushed_tree,
};
#[cfg(test)]
pub(crate) use self::repo_io::{git_stdout_text, git_tree_files, validate_pushed_file_path};
#[cfg(test)]
pub(crate) use self::staging::ReceivePackFileChange;
pub(crate) use self::staging::ReceivePackUpdate;
use self::staging::{apply_receive_pack_update, receive_pack_update_changes_visibility};
use crate::domain::store::{MainPushMode, RepositoryActor, StoredRepository};
use crate::{
    db::RepositoryMutation,
    error::ApiError,
    git::PersistedReceivePackUpdate,
    state::{AppState, repo_config_fingerprint},
};

#[cfg(test)]
pub(crate) async fn persist_receive_pack_update(
    state: &AppState,
    owner: &str,
    repo_name: &str,
    update: ReceivePackUpdate,
) -> Result<PersistedReceivePackUpdate, ApiError> {
    Ok(state
        .metadata
        .mutate_repository(owner, repo_name, move |repo| {
            ensure_receive_pack_base_matches(repo, &update)?;
            apply_receive_pack_update(repo, update)?;
            Ok(RepositoryMutation::new(PersistedReceivePackUpdate::Applied))
        })
        .await?)
}

pub(crate) async fn persist_receive_pack_update_and_promote(
    state: &AppState,
    owner: &str,
    repo_name: &str,
    update: ReceivePackUpdate,
    author_id: &str,
) -> Result<PersistedReceivePackUpdate, ApiError> {
    let uploaded_blobs = update.uploaded_blobs.clone();
    let author_id = author_id.to_string();

    let persisted = state
        .metadata
        .mutate_repository(owner, repo_name, move |repo| {
            let old_snapshot = repo.git_snapshot.clone();
            let mut cleanup_blobs = uploaded_blobs;
            let mut update = update;
            let push_policy = repo.push_policy_for_user_id(&author_id);
            if push_policy.mode == MainPushMode::Denied {
                let message = if push_policy.access.actor == RepositoryActor::Public {
                    "repo membership required"
                } else {
                    "push permission required"
                };
                return Err(ApiError::forbidden(message).into());
            }
            ensure_receive_pack_config_base_matches(repo, &update)?;
            let previous_config = Some(repo.repo_config.clone());
            if !push_policy.access.can_change_file_visibility
                && receive_pack_update_changes_visibility(repo, previous_config.as_ref(), &update)
            {
                return Err(ApiError::forbidden("file visibility permission required").into());
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
        })
        .await?;
    crate::state::best_effort_drain_pending_source_blob_deletions(state).await;
    Ok(persisted)
}

pub(crate) fn apply_request_merge_update(
    repo: &mut StoredRepository,
    update: ReceivePackUpdate,
    maintainer_id: &str,
) -> Result<RepositoryMutation<PersistedReceivePackUpdate>, ApiError> {
    let old_snapshot = repo.git_snapshot.clone();
    let mut cleanup_blobs = update.uploaded_blobs.clone();
    let mut update = update;
    if !repo.is_maintainer_user_id(maintainer_id) {
        return Err(ApiError::forbidden("repo maintainer required"));
    }
    let access = repo.access_for_user_id(maintainer_id);
    ensure_receive_pack_config_base_matches(repo, &update)?;
    let previous_config = Some(repo.repo_config.clone());
    if !access.can_change_file_visibility
        && receive_pack_update_changes_visibility(repo, previous_config.as_ref(), &update)
    {
        return Err(ApiError::forbidden("file visibility permission required"));
    }
    update.previous_config = previous_config;
    ensure_receive_pack_base_matches(repo, &update)?;
    apply_receive_pack_update(repo, update)?;
    cleanup_blobs.extend(old_snapshot);
    Ok(RepositoryMutation::with_source_blob_deletions(
        PersistedReceivePackUpdate::Applied,
        cleanup_blobs,
    ))
}

fn ensure_receive_pack_config_base_matches(
    repo: &StoredRepository,
    update: &ReceivePackUpdate,
) -> Result<(), ApiError> {
    if repo.repo_config == update.config {
        return Ok(());
    }
    if repo_config_fingerprint(&repo.repo_config)? == update.base_config_hash {
        return Ok(());
    }

    Err(ApiError::conflict(
        "repo config changed since review; rerun scope push",
    ))
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
