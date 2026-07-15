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
pub(crate) use self::repo_io::{
    git_segment_manifest_from_repo, git_stdout_text, validate_pushed_file_path,
};
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
use std::time::Instant;

pub(crate) async fn persist_receive_pack_update_and_promote(
    state: &AppState,
    owner: &str,
    repo_name: &str,
    update: ReceivePackUpdate,
    author_id: &str,
) -> Result<PersistedReceivePackUpdate, ApiError> {
    let author_id = author_id.to_string();

    let content_only_candidate = update
        .previous_config
        .as_ref()
        .is_some_and(|previous| previous == &update.config);
    if content_only_candidate
        && let Some(expected_manifest_key) = update
            .base_git_manifest_key
            .as_ref()
            .and_then(|key| key.as_deref())
        && let Some(git_head) = state
            .metadata
            .apply_content_only_push(
                owner,
                repo_name,
                &author_id,
                expected_manifest_key,
                update.clone().into_reviewed_update(),
            )
            .await?
    {
        tracing::info!("committed focused content-only push transaction");
        return Ok(PersistedReceivePackUpdate { git_head });
    }

    let transaction_started = Instant::now();
    let persisted = state
        .metadata
        .mutate_repository(owner, repo_name, move |repo| {
            let domain_started = Instant::now();
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
            update.git_head.change_version = repo.record.change_version.saturating_add(1);
            let committed_git_head = update.git_head.clone();
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
            tracing::info!(
                domain_apply_ms = domain_started.elapsed().as_millis(),
                "applied reviewed push domain transition"
            );
            let persisted = PersistedReceivePackUpdate {
                git_head: committed_git_head,
            };
            Ok(RepositoryMutation::new(persisted))
        })
        .await?;
    tracing::info!(
        database_commit_ms = transaction_started.elapsed().as_millis(),
        "committed reviewed push transaction"
    );
    Ok(persisted)
}

pub(crate) fn apply_request_merge_update(
    repo: &mut StoredRepository,
    update: ReceivePackUpdate,
    maintainer_id: &str,
) -> Result<RepositoryMutation<PersistedReceivePackUpdate>, ApiError> {
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
    let git_head = update.git_head.clone();
    apply_receive_pack_update(repo, update)?;
    Ok(RepositoryMutation::new(PersistedReceivePackUpdate {
        git_head,
    }))
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
    let Some(expected_base_key) = update.base_git_manifest_key.as_ref() else {
        return Ok(());
    };
    let actual_base_key = repo
        .git_head
        .as_ref()
        .map(|head| head.manifest.object_key.as_str());
    if actual_base_key == expected_base_key.as_deref() {
        Ok(())
    } else {
        Err(ApiError::conflict(
            "repo changed since push was reviewed; rerun scope push",
        ))
    }
}
