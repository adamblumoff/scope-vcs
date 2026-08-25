pub(crate) mod main_push;
pub(crate) mod request_ref;

use crate::{
    auth::scope::principal_for_user_id,
    config::GIT_PUSH_TOKEN_PREFIX,
    error::ApiError,
    git::{
        InitialPushCredential, ReceivePackAuthorization, authorize_git_write_token_for_repo,
        authorize_initial_push_for_repo, find_repo_after_git_scope_token, git_credential_error,
        import::PreparedReceivePackUpdate,
        invalid_git_credentials,
        request_refs::{non_request_refs_changed, receive_pack_refs, request_ref_update_from_refs},
        storage::{
            ensure_first_push_receive_pack_staging_repo, ensure_ready_receive_pack_staging_repo,
        },
    },
    push_intents::ValidatedPushIntent,
    repo_access::{ensure_repo_read, find_repo},
    repo_events::RepoChangeReason,
    state::AppState,
};
use scope_domain::repository::{RepoLifecycleState, access::MainPushMode};
use std::{
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

#[derive(Debug)]
pub(crate) enum ReceivePackAccess {
    FirstPush {
        author_id: String,
        push_intent: ValidatedPushIntent,
    },
    ReadyMember {
        author_id: String,
        push_intent: ValidatedPushIntent,
    },
    RequestContributor {
        author_id: String,
    },
}

impl ReceivePackAccess {
    pub(crate) fn author_id(&self) -> &str {
        match self {
            Self::FirstPush { author_id, .. }
            | Self::ReadyMember { author_id, .. }
            | Self::RequestContributor { author_id } => author_id,
        }
    }
}

pub(crate) struct ReceivePreparation {
    pub(crate) access: ReceivePackAccess,
    pub(crate) staging_repo: PathBuf,
    pub(crate) refs_before: Option<Vec<(String, String)>>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ReceiveCompletion {
    NoChange,
    RequestRevision,
    MainPush,
}

pub(crate) async fn authorize(
    state: &AppState,
    owner: &str,
    repo_name: &str,
    authorization: ReceivePackAuthorization,
    push_intent_secret: Option<&str>,
) -> Result<ReceivePackAccess, ApiError> {
    match authorization {
        ReceivePackAuthorization::ScopeToken { secret } => {
            let push_intent = required_push_intent(state, push_intent_secret)?;
            let repo = find_repo_after_git_scope_token(state, owner, repo_name).await?;
            let credential = if secret.starts_with(GIT_PUSH_TOKEN_PREFIX) {
                InitialPushCredential::GitPushToken { secret }
            } else {
                InitialPushCredential::FirstPushToken { secret }
            };
            if repo.is_waiting_for_first_push() {
                authorize_initial_push_for_repo(&repo, &credential)
                    .map_err(git_credential_error)?;
                let author_id = repo.record.owner_user_id.clone();
                push_intent.ensure_repo_user(&repo.record.id, &author_id)?;
                return Ok(ReceivePackAccess::FirstPush {
                    author_id,
                    push_intent,
                });
            }
            match repo.record.lifecycle_state {
                RepoLifecycleState::AwaitingFirstPush => Err(ApiError::conflict(
                    "repo is awaiting its first push and cannot receive another push",
                )),
                RepoLifecycleState::Ready => match credential {
                    InitialPushCredential::GitPushToken { secret } => {
                        let author_id = authorize_git_write_token_for_repo(&repo, &secret)
                            .map_err(git_credential_error)?;
                        push_intent.ensure_repo_user(&repo.record.id, &author_id)?;
                        Ok(ReceivePackAccess::ReadyMember {
                            author_id,
                            push_intent,
                        })
                    }
                    InitialPushCredential::FirstPushToken { .. } => Err(invalid_git_credentials()),
                },
            }
        }
        ReceivePackAuthorization::ScopeUser(user) => {
            if let Some(secret) = push_intent_secret
                && let Ok(push_intent) = state.validate_push_intent_secret(secret)
                && let Some(context) = state
                    .metadata
                    .repositories()
                    .git_push_context(owner, repo_name, &user.id)
                    .await?
                && context.lifecycle_state == RepoLifecycleState::Ready
                && context.access.can_push
            {
                push_intent.ensure_repo_user(&context.repo_id, &user.id)?;
                return Ok(ReceivePackAccess::ReadyMember {
                    author_id: user.id,
                    push_intent,
                });
            }
            let repo = find_repo(state, owner, repo_name).await?;
            let principal = principal_for_user_id(&repo, &user.id);
            let push_policy = repo.push_policy_for_user_id(&user.id);
            let author_id = user.id.clone();
            if push_policy.mode == MainPushMode::FirstPush {
                let push_intent = required_push_intent(state, push_intent_secret)?;
                push_intent.ensure_repo_user(&repo.record.id, &author_id)?;
                return Ok(ReceivePackAccess::FirstPush {
                    author_id,
                    push_intent,
                });
            }
            if push_policy.mode == MainPushMode::Denied {
                if repo.record.lifecycle_state == RepoLifecycleState::Ready
                    && actor_can_receive_request_push(
                        state,
                        &repo,
                        &principal,
                        &author_id,
                        push_policy.access,
                    )
                    .await?
                {
                    return Ok(ReceivePackAccess::RequestContributor { author_id });
                }
                return Err(ApiError::not_found(format!(
                    "repo {owner}/{repo_name} not found"
                )));
            }
            match repo.record.lifecycle_state {
                RepoLifecycleState::AwaitingFirstPush => Err(ApiError::conflict(
                    "repo is awaiting its first push and cannot receive another push",
                )),
                RepoLifecycleState::Ready => {
                    if let Some(secret) = push_intent_secret {
                        match state.validate_push_intent_secret(secret) {
                            Ok(push_intent) => {
                                push_intent.ensure_repo_user(&repo.record.id, &author_id)?;
                                return Ok(ReceivePackAccess::ReadyMember {
                                    author_id,
                                    push_intent,
                                });
                            }
                            Err(error) => {
                                if actor_can_receive_request_push(
                                    state,
                                    &repo,
                                    &principal,
                                    &author_id,
                                    push_policy.access,
                                )
                                .await?
                                {
                                    return Ok(ReceivePackAccess::RequestContributor { author_id });
                                }
                                return Err(error);
                            }
                        }
                    }
                    if actor_can_receive_request_push(
                        state,
                        &repo,
                        &principal,
                        &author_id,
                        push_policy.access,
                    )
                    .await?
                    {
                        Ok(ReceivePackAccess::RequestContributor { author_id })
                    } else {
                        Err(ApiError::forbidden("valid Scope push intent required"))
                    }
                }
            }
        }
    }
}

pub(crate) async fn prepare(
    state: &AppState,
    owner: &str,
    repo_name: &str,
    access: ReceivePackAccess,
    advertisement_only: bool,
) -> Result<ReceivePreparation, ApiError> {
    let staging_repo = match &access {
        ReceivePackAccess::FirstPush { .. } => {
            ensure_first_push_receive_pack_staging_repo(state, owner, repo_name)?
        }
        ReceivePackAccess::ReadyMember { author_id, .. } => {
            let staging =
                ensure_ready_receive_pack_staging_repo(state, owner, repo_name, author_id).await?;
            if let Err(error) = request_ref::seed_editable_request_refs(
                state, owner, repo_name, author_id, &staging,
            )
            .await
            {
                let _ = crate::git::storage::remove_dir_if_exists(&staging);
                return Err(error);
            }
            staging
        }
        ReceivePackAccess::RequestContributor { author_id } => {
            request_ref::prepare_request_staging_repo(state, owner, repo_name, author_id).await?
        }
    };
    let refs_before = if advertisement_only {
        None
    } else {
        match receive_pack_refs(&staging_repo) {
            Ok(refs) => Some(refs),
            Err(error) => {
                let _ = crate::git::storage::remove_dir_if_exists(&staging_repo);
                return Err(error);
            }
        }
    };
    Ok(ReceivePreparation {
        access,
        staging_repo,
        refs_before,
    })
}

pub(crate) async fn complete(
    state: &AppState,
    owner: &str,
    repo_name: &str,
    staging_repo: &Path,
    preparation: ReceivePreparation,
    receive_elapsed: Duration,
) -> Result<ReceiveCompletion, ApiError> {
    let refs_after = receive_pack_refs(staging_repo)?;
    let refs_before = preparation
        .refs_before
        .ok_or_else(|| ApiError::internal_message("missing refs before receive-pack"))?;
    if refs_before == refs_after {
        tracing::debug!(
            owner,
            repo = repo_name,
            receive_ms = receive_elapsed.as_millis(),
            "git receive-pack left refs unchanged"
        );
        return Ok(ReceiveCompletion::NoChange);
    }

    if let Some(update) = request_ref_update_from_refs(&refs_before, &refs_after)? {
        if non_request_refs_changed(&refs_before, &refs_after) {
            return Err(ApiError::bad_request(
                "Scope accepts either one request ref update or one main update",
            ));
        }
        let author_id = match &preparation.access {
            ReceivePackAccess::FirstPush { .. } => {
                return Err(ApiError::bad_request(
                    "request refs cannot be pushed during first push",
                ));
            }
            ReceivePackAccess::ReadyMember { author_id, .. }
            | ReceivePackAccess::RequestContributor { author_id } => author_id,
        };
        request_ref::persist_request_ref_revision(
            state,
            owner,
            repo_name,
            author_id,
            staging_repo,
            update,
        )
        .await?;
        tracing::info!(
            owner,
            repo = repo_name,
            receive_ms = receive_elapsed.as_millis(),
            "git receive-pack request ref persisted"
        );
        return Ok(ReceiveCompletion::RequestRevision);
    }

    if matches!(
        &preparation.access,
        ReceivePackAccess::RequestContributor { .. }
    ) {
        return Err(ApiError::bad_request(
            "public contributors can only push named request branches",
        ));
    }
    complete_main_push(
        state,
        owner,
        repo_name,
        staging_repo,
        preparation.access,
        receive_elapsed,
    )
    .await?;
    Ok(ReceiveCompletion::MainPush)
}

async fn complete_main_push(
    state: &AppState,
    owner: &str,
    repo_name: &str,
    staging_repo: &Path,
    access: ReceivePackAccess,
    receive_elapsed: Duration,
) -> Result<(), ApiError> {
    let first_push = matches!(&access, ReceivePackAccess::FirstPush { .. });
    let author_id = access.author_id().to_string();
    let import_started_at = Instant::now();
    let (prepared, change_count): (PreparedReceivePackUpdate, usize) =
        main_push::prepare_main_push(state, owner, repo_name, staging_repo, &access).await?;
    let durable_objects = prepared.durable_objects.clone();
    let committed_git_head =
        match main_push::persist_main_push(state, owner, repo_name, prepared, &author_id).await {
            Ok(git_head) => git_head,
            Err(error) => {
                crate::use_cases::content_cleanup::best_effort_cleanup_rollback_source_blobs(
                    state,
                    &durable_objects,
                )
                .await;
                return Err(error);
            }
        };
    let event = if first_push {
        state
            .product_analytics
            .capture(crate::product_analytics::ProductEvent::repository_initialized(&author_id));
        RepoChangeReason::FirstPushApplied
    } else {
        RepoChangeReason::PushReceived
    };
    state
        .publish_repo_change(
            &scope_domain::repository::repo_id(owner, repo_name),
            committed_git_head.change_version,
            event,
        )
        .await;
    tracing::info!(
        owner,
        repo = repo_name,
        receive_ms = receive_elapsed.as_millis(),
        import_ms = import_started_at.elapsed().as_millis(),
        change_count,
        first_push,
        "git receive-pack main update persisted"
    );
    best_effort_sync_cache(
        state,
        owner,
        repo_name,
        &author_id,
        staging_repo,
        &committed_git_head,
    )
    .await;
    Ok(())
}

async fn best_effort_sync_cache(
    state: &AppState,
    owner: &str,
    repo_name: &str,
    author_id: &str,
    staging_repo: &Path,
    committed_git_head: &scope_domain::repository::git::GitHead,
) {
    match state
        .metadata
        .repositories()
        .git_push_context(owner, repo_name, author_id)
        .await
    {
        Ok(Some(repo)) => {
            let is_still_current = repo.git_head.as_ref().is_some_and(|head| {
                head.manifest.content_ref == committed_git_head.manifest.content_ref
            });
            if is_still_current
                && let Err(error) = state.repository_engine.sync_after_push(
                    &scope_domain::repository::repo_id(owner, repo_name),
                    staging_repo,
                    &committed_git_head.head_oid,
                    committed_git_head.push_sequence,
                )
            {
                tracing::warn!(
                    owner,
                    repo = repo_name,
                    error = %error.operator_diagnostic(),
                    "push committed but repository Git cache synchronization failed"
                );
            }
        }
        Ok(None) => tracing::warn!(
            owner,
            repo = repo_name,
            "push committed but repository context was unavailable"
        ),
        Err(error) => tracing::warn!(
            owner,
            repo = repo_name,
            error = %error.message,
            "push committed but post-commit context refresh failed"
        ),
    }
}

fn required_push_intent(
    state: &AppState,
    secret: Option<&str>,
) -> Result<ValidatedPushIntent, ApiError> {
    let secret = secret.ok_or_else(|| ApiError::forbidden("valid Scope push intent required"))?;
    state.validate_push_intent_secret(secret)
}

async fn actor_can_receive_request_push(
    state: &AppState,
    repo: &scope_domain::repository::Repository,
    principal: &scope_domain::policy::Principal,
    author_id: &str,
    access: scope_domain::repository::access::RepositoryAccess,
) -> Result<bool, ApiError> {
    ensure_repo_read(state, repo, principal)?;
    request_ref::actor_has_open_editable_request(state, &repo.record.id, author_id, access).await
}
