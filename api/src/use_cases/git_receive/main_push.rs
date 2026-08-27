use crate::{
    error::ApiError,
    git::import::{
        PreparedReceivePackUpdate, ReceivePackUpdate, apply_receive_pack_update,
        receive_pack_update_from_staging_repo, reviewed_update_from_staging_repo,
    },
    state::AppState,
};
use scope_domain::{
    error::DomainError,
    repo_config::repo_config_fingerprint as domain_repo_config_fingerprint,
    repository::{Repository, git::GitHead},
    reviewed_updates::content::{ReviewedUpdateAuthorization, authorize_reviewed_update},
    runs::trigger::PushTriggerInput,
};
use scope_git_storage::StagedGitSegment;
use scope_postgres::db::RepositoryGitWriteLease;
use scope_postgres::db::RepositoryMutation;
use std::{path::Path, time::Instant};

use super::ReceivePackAccess;

pub(crate) struct PersistedGitPush {
    pub(crate) head: GitHead,
    pub(crate) staged_segment: StagedGitSegment,
    pub(crate) write_lease: RepositoryGitWriteLease,
}

impl std::fmt::Debug for PersistedGitPush {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("PersistedGitPush")
            .field("head", &self.head)
            .field("segment", &self.staged_segment.segment)
            .finish_non_exhaustive()
    }
}

pub(super) async fn prepare_main_push(
    state: &AppState,
    owner: &str,
    repo_name: &str,
    staging_repo: &Path,
    access: &ReceivePackAccess,
) -> Result<(PreparedReceivePackUpdate, usize), ApiError> {
    let (author_id, push_intent, first_push) = match access {
        ReceivePackAccess::FirstPush {
            author_id,
            push_intent,
        } => (author_id, push_intent, true),
        ReceivePackAccess::ReadyMember {
            author_id,
            push_intent,
        } => (author_id, push_intent, false),
        ReceivePackAccess::RequestContributor { .. } => {
            return Err(ApiError::bad_request(
                "public contributors can only push named request branches",
            ));
        }
    };
    let mut prepared = if first_push {
        reviewed_update_from_staging_repo(
            state,
            owner,
            repo_name,
            staging_repo,
            author_id,
            push_intent.config.clone(),
        )
        .await?
    } else {
        receive_pack_update_from_staging_repo(
            state,
            owner,
            repo_name,
            staging_repo,
            author_id,
            push_intent.config.clone(),
        )
        .await?
    };
    let durable_objects = prepared.durable_objects.clone();
    prepared.base_git_manifest_ref = Some(match push_intent.base_for_head(&prepared.head_oid) {
        Ok(base) => base,
        Err(error) => {
            let repository_id = scope_domain::repository::repo_id(owner, repo_name);
            crate::git::import::best_effort_delete_staged_git_segment(
                state,
                &repository_id,
                &prepared.staged_segment,
            )
            .await;
            crate::use_cases::content_cleanup::best_effort_cleanup_rollback_source_blobs(
                state,
                &durable_objects,
            )
            .await;
            prepared.fence.release().await;
            prepared.write_lease.release().await;
            return Err(error);
        }
    });
    prepared.base_config_hash = push_intent.base_config_hash.clone();
    let change_count = prepared.changes.len();
    Ok((prepared, change_count))
}

pub(crate) async fn persist_main_push(
    state: &AppState,
    owner: &str,
    repo_name: &str,
    prepared: PreparedReceivePackUpdate,
    author_id: &str,
) -> Result<PersistedGitPush, ApiError> {
    let PreparedReceivePackUpdate {
        update,
        fence,
        staged_segment,
        write_lease,
    } = prepared;
    let repository_id = scope_domain::repository::repo_id(owner, repo_name);
    let durable_objects = update.durable_objects.clone();
    let now_unix = match crate::persistence::unix_now() {
        Ok(now) => now,
        Err(error) => {
            cleanup_failed_persist(
                state,
                &repository_id,
                &staged_segment,
                &durable_objects,
                fence,
                write_lease,
            )
            .await;
            return Err(error);
        }
    };
    let author_id = author_id.to_string();
    let workflow_catalog = update.workflow_catalog.clone();
    let push_trigger_input = PushTriggerInput::from(&workflow_catalog);

    let content_only_candidate = update
        .previous_config
        .as_ref()
        .is_some_and(|previous| previous == &update.config);
    if content_only_candidate
        && let Some(expected_manifest_ref) = update
            .base_git_manifest_ref
            .as_ref()
            .and_then(Option::as_ref)
        && let Some(git_head) = match state
            .metadata
            .repositories()
            .apply_content_only_push(
                scope_postgres::db::ApplyContentOnlyPushCommand {
                    owner: owner.to_string(),
                    name: repo_name.to_string(),
                    author_id: author_id.clone(),
                    expected_manifest_ref: expected_manifest_ref.clone(),
                    update: update.clone().into_reviewed_update(),
                    workflow_catalog: workflow_catalog.clone(),
                    push_trigger_input: push_trigger_input.clone(),
                    landing_file_mutation: update.landing_file_mutation.clone(),
                    now_unix,
                },
                &crate::persistence_ids::generate_persistence_id,
            )
            .await
        {
            Ok(result) => result,
            Err(error) => {
                cleanup_failed_persist(
                    state,
                    &repository_id,
                    &staged_segment,
                    &durable_objects,
                    fence,
                    write_lease,
                )
                .await;
                return Err(error.into());
            }
        }
    {
        tracing::info!("committed focused content-only push transaction");
        fence.release().await;
        return Ok(PersistedGitPush {
            head: git_head,
            staged_segment,
            write_lease,
        });
    }

    let transaction_started = Instant::now();
    let git_head = state
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
                authorize_reviewed_update(ReviewedUpdateAuthorization {
                    access: push_policy.access,
                    push_mode: push_policy.mode,
                    current_config: &repo.repo_config,
                    proposed_config: &repo.repo_config,
                })?;
                update.git_head.change_version = repo.record.change_version.saturating_add(1);
                let committed_git_head = update.git_head.clone();
                ensure_receive_pack_config_base_matches(repo, &update)?;
                authorize_reviewed_update(ReviewedUpdateAuthorization {
                    access: push_policy.access,
                    push_mode: push_policy.mode,
                    current_config: &repo.repo_config,
                    proposed_config: &update.config,
                })?;
                update.previous_config = Some(repo.repo_config.clone());
                ensure_receive_pack_base_matches(repo, &update)?;
                let landing_file_mutation = update.landing_file_mutation.clone();
                apply_receive_pack_update(repo, update)?;
                tracing::info!(
                    domain_apply_ms = domain_started.elapsed().as_millis(),
                    "applied reviewed push domain transition"
                );
                let workflow_catalog = workflow_catalog
                    .rebind_source_change_version(
                        &repo.record.id,
                        &committed_git_head.head_oid,
                        committed_git_head.change_version,
                    )
                    .map_err(DomainError::invariant_violation)?;
                Ok(RepositoryMutation::with_push_trigger_input(
                    committed_git_head,
                    push_trigger_input,
                    landing_file_mutation,
                    workflow_catalog,
                ))
            },
        )
        .await;
    let git_head = match git_head {
        Ok(git_head) => git_head,
        Err(error) => {
            cleanup_failed_persist(
                state,
                &repository_id,
                &staged_segment,
                &durable_objects,
                fence,
                write_lease,
            )
            .await;
            return Err(error.into());
        }
    };
    tracing::info!(
        database_commit_ms = transaction_started.elapsed().as_millis(),
        "committed reviewed push transaction"
    );
    fence.release().await;
    Ok(PersistedGitPush {
        head: git_head,
        staged_segment,
        write_lease,
    })
}

async fn cleanup_failed_persist(
    state: &AppState,
    repository_id: &str,
    staged_segment: &StagedGitSegment,
    durable_objects: &[scope_domain::content::SourceBlob],
    fence: scope_postgres::db::ContentRefFence,
    write_lease: RepositoryGitWriteLease,
) {
    crate::git::import::best_effort_delete_staged_git_segment(state, repository_id, staged_segment)
        .await;
    crate::use_cases::content_cleanup::best_effort_cleanup_rollback_source_blobs(
        state,
        durable_objects,
    )
    .await;
    fence.release().await;
    write_lease.release().await;
}

fn ensure_receive_pack_config_base_matches(
    repo: &Repository,
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
    repo: &Repository,
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
