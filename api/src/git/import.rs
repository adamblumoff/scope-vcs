mod artifacts;
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
pub(crate) use self::staging::ReceivePackUpdate;
use self::staging::stage_receive_pack_update_for_access;
#[cfg(test)]
pub(crate) use self::staging::{ReceivePackFileChange, stage_receive_pack_update};
use crate::domain::store::{
    FirstPushTokenStatus, PendingImport, RepoPublicationState, RepositoryActor,
};
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
) -> Result<u64, ApiError> {
    let now = unix_now()?;
    let credential = credential.clone();
    state
        .metadata
        .mutate_repository(owner, repo_name, move |repo| {
            if !repo.is_waiting_for_first_push() {
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
            repo.record.publication_state = RepoPublicationState::Unpublished;
            repo.bump_change_version();
            Ok(RepositoryMutation::new(repo.record.change_version))
        })
}

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
            if stage_receive_pack_update_for_access(repo, update, true)?.is_some() {
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
    author_id: &str,
) -> Result<PersistedReceivePackUpdate, ApiError> {
    let uploaded_blobs = update.uploaded_blobs.clone();
    let author_id = author_id.to_string();

    let persisted = state
        .metadata
        .mutate_repository(owner, repo_name, move |repo| {
            let old_snapshot = repo.git_snapshot.clone();
            let mut cleanup_blobs = uploaded_blobs;
            let access = repo.access_for_user_id(&author_id);
            if !access.can_push {
                let message = if access.actor == RepositoryActor::Public {
                    "repo membership required"
                } else {
                    "push permission required"
                };
                return Err(ApiError::forbidden(message));
            }
            let persisted =
                if stage_receive_pack_update_for_access(repo, update, access.can_apply_changes)?
                    .is_some()
                {
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
