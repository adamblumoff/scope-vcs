mod artifacts;
mod repo_io;
mod staging;

pub(crate) use self::artifacts::{
    receive_pack_update_from_staging_repo, request_merge_update_from_staging_repo,
    reviewed_update_from_staging_repo,
};
#[cfg(test)]
pub(crate) use self::repo_io::{
    git_push_from_repo, git_refs, git_stdout_text, validate_pushed_file_path,
};
pub(crate) use self::repo_io::{
    git_snapshot_from_ref, run_git, run_git_output, run_git_output_bounded, safe_repo_key,
    validate_pushed_commit_range, validate_pushed_tree,
};
#[cfg(test)]
pub(crate) use self::staging::ReceivePackFileChange;
pub(crate) use self::staging::ReceivePackUpdate;
use self::staging::{apply_receive_pack_update, receive_pack_update_changes_visibility};
use crate::{error::ApiError, git::PersistedReceivePackUpdate, state::AppState};
use scope_domain::{
    error::DomainError,
    repo_config::repo_config_fingerprint as domain_repo_config_fingerprint,
    store::{MainPushMode, RepositoryActor, StoredRepository},
};
use scope_postgres::db::RepositoryMutation;
use std::time::Instant;

pub(crate) async fn persist_main_push_update_and_promote(
    state: &AppState,
    owner: &str,
    repo_name: &str,
    mut update: ReceivePackUpdate,
    author_id: &str,
) -> Result<PersistedReceivePackUpdate, ApiError> {
    let now_unix = crate::persistence::unix_now()?;
    let author_id = author_id.to_string();
    let push_trigger_input = update.push_trigger_input.take().ok_or_else(|| {
        ApiError::internal_message("main push is missing its pinned trigger input")
    })?;

    let content_only_candidate = update
        .previous_config
        .as_ref()
        .is_some_and(|previous| previous == &update.config);
    if content_only_candidate
        && let Some(expected_manifest_ref) = update
            .base_git_manifest_ref
            .as_ref()
            .and_then(Option::as_ref)
        && let Some(git_head) = state
            .metadata
            .repositories()
            .apply_content_only_push(
                scope_postgres::db::ApplyContentOnlyPushCommand {
                    owner: owner.to_string(),
                    name: repo_name.to_string(),
                    author_id: author_id.clone(),
                    expected_manifest_ref: expected_manifest_ref.clone(),
                    update: update.clone().into_reviewed_update(),
                    push_trigger_input: push_trigger_input.clone(),
                    now_unix,
                },
                &crate::persistence_ids::generate_persistence_id,
            )
            .await?
    {
        tracing::info!("committed focused content-only push transaction");
        return Ok(PersistedReceivePackUpdate { git_head });
    }

    let transaction_started = Instant::now();
    let persisted = state
        .metadata
        .repositories()
        .mutate_repository(
            owner,
            repo_name,
            now_unix,
            &crate::persistence_ids::generate_persistence_id,
            move |repo| {
                let domain_started = Instant::now();
                let mut update = update;
                let push_policy = repo.push_policy_for_user_id(&author_id);
                if push_policy.mode == MainPushMode::Denied {
                    let message = if push_policy.access.actor == RepositoryActor::Public {
                        "repo membership required"
                    } else {
                        "push permission required"
                    };
                    return Err(DomainError::forbidden(message));
                }
                update.git_head.change_version = repo.record.change_version.saturating_add(1);
                let committed_git_head = update.git_head.clone();
                ensure_receive_pack_config_base_matches(repo, &update)?;
                let previous_config = Some(repo.repo_config.clone());
                if !push_policy.access.can_change_file_visibility
                    && receive_pack_update_changes_visibility(
                        repo,
                        previous_config.as_ref(),
                        &update,
                    )
                {
                    return Err(DomainError::forbidden(
                        "file visibility permission required",
                    ));
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
                Ok(RepositoryMutation::with_push_trigger_input(
                    persisted,
                    push_trigger_input,
                ))
            },
        )
        .await?;
    tracing::info!(
        database_commit_ms = transaction_started.elapsed().as_millis(),
        "committed reviewed push transaction"
    );
    Ok(persisted)
}

fn ensure_receive_pack_config_base_matches(
    repo: &StoredRepository,
    update: &ReceivePackUpdate,
) -> Result<(), DomainError> {
    if repo.repo_config == update.config {
        return Ok(());
    }
    if domain_repo_config_fingerprint(&repo.repo_config)
        .map_err(DomainError::invariant_violation)?
        == update.base_config_hash
    {
        return Ok(());
    }

    Err(DomainError::conflict(
        "repo config changed since review; rerun scope push",
    ))
}

fn ensure_receive_pack_base_matches(
    repo: &StoredRepository,
    update: &ReceivePackUpdate,
) -> Result<(), DomainError> {
    let Some(expected_base_ref) = update.base_git_manifest_ref.as_ref() else {
        return Ok(());
    };
    let actual_base_ref = repo
        .git_head
        .as_ref()
        .map(|head| &head.manifest.content_ref);
    if actual_base_ref == expected_base_ref.as_ref() {
        Ok(())
    } else {
        Err(DomainError::conflict(
            "repo changed since push was reviewed; rerun scope push",
        ))
    }
}
