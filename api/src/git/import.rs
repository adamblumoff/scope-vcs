use crate::domain::projection_views::pending_scope_path;
mod diff;
mod repo_io;
mod staging;

use self::repo_io::{
    describe_refs, git_snapshot_from_repo, git_tree_entries, pushed_commit_message,
    read_git_tree_blob,
};
pub(crate) use self::repo_io::{git_refs, git_stdout_text, git_tree_files, run_git, safe_repo_key};
#[cfg(test)]
pub(crate) use self::repo_io::{run_git_output, validate_pushed_file_path};
#[cfg(test)]
pub(crate) use self::staging::stage_receive_pack_update;
pub(crate) use self::staging::{
    ReceivePackFileChange, ReceivePackUpdate, apply_receive_pack_update, ensure_default_branch,
    validate_staged_update_policy,
};
use self::staging::{source_content_matches, stage_receive_pack_update_with_store};
use crate::domain::store::{FirstPushTokenStatus, PendingImport, RepoPublicationState};
use crate::{
    db::RepositoryMutation,
    error::ApiError,
    git::{
        InitialPushCredential, PersistedReceivePackUpdate, authorize_first_push_token_for_repo,
        authorize_git_push_token_for_repo,
    },
    object_store::put_repo_object,
    persistence::unix_now,
    state::AppState,
    state::{find_repo, live_tree},
};
use std::{collections::BTreeSet, path::Path as FsPath};

pub(crate) fn pending_import_from_staging_repo(
    state: &AppState,
    owner: &str,
    repo_name: &str,
    staging_repo: &FsPath,
) -> Result<PendingImport, ApiError> {
    let refs = git_refs(staging_repo)?;
    if refs.len() != 1 {
        return Err(ApiError::bad_request(format!(
            "push must create exactly one branch and no tags; found {}",
            describe_refs(&refs)
        )));
    }
    let (refname, head_oid) = refs.into_iter().next().expect("length checked");
    let Some(default_branch) = refname.strip_prefix("refs/heads/") else {
        return Err(ApiError::bad_request("only branch pushes are supported"));
    };
    ensure_default_branch(default_branch)?;
    let tree_oid = git_stdout_text(
        staging_repo,
        &["rev-parse", &format!("{head_oid}^{{tree}}")],
        "reading pushed tree",
    )?
    .trim()
    .to_string();
    let imported_at_unix = unix_now()?;
    let repo_id = crate::domain::store::repo_id(owner, repo_name);
    let files = git_tree_files(state, &repo_id, staging_repo, &head_oid)?;
    let uploaded_file_blobs = files
        .iter()
        .map(|file| file.blob.clone())
        .collect::<Vec<_>>();
    let git_snapshot = match git_snapshot_from_repo(state, &repo_id, staging_repo) {
        Ok(snapshot) => snapshot,
        Err(error) => {
            crate::state::best_effort_cleanup_rollback_source_blobs(state, &uploaded_file_blobs);
            return Err(error);
        }
    };

    Ok(PendingImport {
        default_branch: default_branch.to_string(),
        head_oid,
        tree_oid,
        imported_at_unix,
        git_snapshot,
        files,
    })
}

pub(crate) fn receive_pack_update_from_staging_repo(
    state: &AppState,
    owner: &str,
    repo_name: &str,
    staging_repo: &FsPath,
    author_id: &str,
) -> Result<ReceivePackUpdate, ApiError> {
    let refs = git_refs(staging_repo)?;
    if refs.len() != 1 {
        return Err(ApiError::bad_request(format!(
            "push must update exactly one branch and no tags; found {}",
            describe_refs(&refs)
        )));
    }
    let (branch, head_oid) = refs.into_iter().next().expect("length checked");
    ensure_default_branch(&branch)?;
    let repo = find_repo(state, owner, repo_name)?;
    if repo.record.publication_state != RepoPublicationState::Published {
        return Err(ApiError::conflict("repo must be published before push"));
    }
    let repo_id = crate::domain::store::repo_id(owner, repo_name);
    let message = pushed_commit_message(staging_repo, &head_oid)?;
    let live_tree = live_tree(&repo);
    let pushed_entries = git_tree_entries(staging_repo, &head_oid)?;
    let mut changes = Vec::new();
    let mut uploaded_file_blobs = Vec::new();
    let mut pushed_paths = BTreeSet::new();

    for entry in pushed_entries {
        let path = match pending_scope_path(&entry.path) {
            Ok(path) => path,
            Err(error) => {
                crate::state::best_effort_cleanup_rollback_source_blobs(
                    state,
                    &uploaded_file_blobs,
                );
                return Err(error);
            }
        };
        pushed_paths.insert(path.clone());
        let live_content = live_tree.get(&path);
        if live_content.is_some_and(|blob| {
            blob.git_oid == entry.oid && blob.size_bytes == entry.size_bytes as u64
        }) {
            continue;
        }

        let content = match read_git_tree_blob(staging_repo, &entry.oid, &entry.path) {
            Ok(content) => content,
            Err(error) => {
                crate::state::best_effort_cleanup_rollback_source_blobs(
                    state,
                    &uploaded_file_blobs,
                );
                return Err(error);
            }
        };
        let new_content =
            match put_repo_object(state.object_store.as_ref(), &repo_id, "blobs", &content) {
                Ok(blob) => blob,
                Err(error) => {
                    crate::state::best_effort_cleanup_rollback_source_blobs(
                        state,
                        &uploaded_file_blobs,
                    );
                    return Err(error);
                }
            };
        uploaded_file_blobs.push(new_content.clone());
        if !source_content_matches(live_content, Some(&new_content)) {
            changes.push(ReceivePackFileChange {
                path,
                content: Some(new_content),
            });
        }
    }
    for path in live_tree.keys() {
        if !pushed_paths.contains(path) {
            changes.push(ReceivePackFileChange {
                path: path.clone(),
                content: None,
            });
        }
    }
    if changes.is_empty() {
        return Err(ApiError::bad_request(
            "receive-pack update did not change the live tree",
        ));
    }
    let git_snapshot = match git_snapshot_from_repo(state, &repo_id, staging_repo) {
        Ok(snapshot) => snapshot,
        Err(error) => {
            crate::state::best_effort_cleanup_rollback_source_blobs(state, &uploaded_file_blobs);
            return Err(error);
        }
    };

    Ok(ReceivePackUpdate {
        branch,
        author_id: author_id.to_string(),
        message,
        git_snapshot,
        uploaded_blobs: uploaded_file_blobs,
        changes,
    })
}

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
