mod artifacts;
mod diff;
mod repo_io;
mod staging;

pub(crate) use self::artifacts::{
    pending_import_from_staging_repo, receive_pack_update_from_staging_repo,
};
pub(crate) use self::repo_io::{git_refs, run_git, safe_repo_key};
#[cfg(test)]
pub(crate) use self::repo_io::{
    git_stdout_text, git_tree_files, run_git_output, validate_pushed_file_path,
};
use self::staging::stage_receive_pack_update_with_store;
#[cfg(test)]
pub(crate) use self::staging::{ReceivePackFileChange, stage_receive_pack_update};
pub(crate) use self::staging::{
    ReceivePackUpdate, apply_receive_pack_update, validate_staged_update_policy,
};
use crate::domain::store::{FirstPushTokenStatus, PendingImport, RepoPublicationState};
use crate::{
    db::RepositoryMutation,
    error::ApiError,
    git::{
        InitialPushCredential, PersistedReceivePackUpdate, authorize_first_push_token_for_repo,
        authorize_git_push_token_for_repo,
    },
    persistence::unix_now,
    state::AppState,
};

pub(crate) fn persist_pending_import(
    state: &AppState,
    owner: &str,
    repo_name: &str,
    credential: &InitialPushCredential,
    import: PendingImport,
) -> Result<(), ApiError> {
    let now = unix_now()?;
    let credential = credential.clone();
    state
        .metadata
        .mutate_repository(owner, repo_name, move |repo| {
            if repo.record.publication_state != RepoPublicationState::PendingFirstPush {
                return Err(ApiError::conflict(
                    "repo is not waiting for an initial Git push",
                ));
            }
            if repo.pending_import.is_some() {
                return Err(ApiError::conflict("repo already has a pending import"));
            }
            match credential {
                InitialPushCredential::FirstPushToken { secret } => {
                    authorize_first_push_token_for_repo(repo, &secret)?;
                }
                InitialPushCredential::GitPushToken { secret } => {
                    authorize_git_push_token_for_repo(repo, &secret)?;
                }
            }
            if let Some(token) = repo.first_push_token.as_mut()
                && token.status_at(now) == FirstPushTokenStatus::Active
            {
                token.used_at_unix = Some(now);
            }
            repo.pending_import = Some(import);
            repo.record.publication_state = RepoPublicationState::PendingPublish;
            Ok(RepositoryMutation::new(()))
        })
}

#[cfg(test)]
pub(crate) fn persist_receive_pack_update(
    state: &AppState,
    owner: &str,
    repo_name: &str,
    update: ReceivePackUpdate,
) -> Result<PersistedReceivePackUpdate, ApiError> {
    let store = state.object_store.clone();
    state
        .metadata
        .mutate_repository(owner, repo_name, move |repo| {
            if stage_receive_pack_update_with_store(repo, update, store.as_ref())?.is_some() {
                Ok(RepositoryMutation::new(PersistedReceivePackUpdate::Staged))
            } else {
                Ok(RepositoryMutation::new(PersistedReceivePackUpdate::Applied))
            }
        })
}

pub(crate) fn persist_receive_pack_update_and_promote(
    state: &AppState,
    owner: &str,
    repo_name: &str,
    update: ReceivePackUpdate,
) -> Result<PersistedReceivePackUpdate, ApiError> {
    let uploaded_blobs = update.uploaded_blobs.clone();
    let store = state.object_store.clone();

    let persisted = state
        .metadata
        .mutate_repository(owner, repo_name, move |repo| {
            let old_snapshot = repo.git_snapshot.clone();
            let mut cleanup_blobs = uploaded_blobs;
            let persisted =
                if stage_receive_pack_update_with_store(repo, update, store.as_ref())?.is_some() {
                    PersistedReceivePackUpdate::Staged
                } else {
                    cleanup_blobs.extend(old_snapshot);
                    PersistedReceivePackUpdate::Applied
                };
            Ok(RepositoryMutation::with_source_blob_deletions(
                persisted,
                cleanup_blobs,
            ))
        })?;
    crate::state::best_effort_drain_pending_source_blob_deletions(state);
    Ok(persisted)
}
