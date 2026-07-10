use super::repo_io::{
    describe_refs, git_refs, git_snapshot_from_repo, git_tree_blob_contents, git_tree_entries,
    pushed_commit_message, put_git_blob_contents,
};
use super::staging::{ReceivePackFileChange, ReceivePackUpdate, ensure_default_branch};
use crate::domain::projection_views::repo_scope_path;
use crate::domain::repo_config::RepoConfig;
use crate::domain::reviewed_updates::source_content_matches;
use crate::domain::store::RepoPublicationState;
use crate::{error::ApiError, state::AppState, state::find_repo};
use std::{collections::BTreeSet, path::Path as FsPath};

pub(crate) async fn receive_pack_update_from_staging_repo(
    state: &AppState,
    owner: &str,
    repo_name: &str,
    staging_repo: &FsPath,
    author_id: &str,
    config: RepoConfig,
) -> Result<ReceivePackUpdate, ApiError> {
    let repo = find_repo(state, owner, repo_name).await?;
    if repo.record.publication_state != RepoPublicationState::Published {
        return Err(ApiError::conflict("repo must be published before push"));
    }
    reviewed_update_from_staging_repo(state, owner, repo_name, staging_repo, author_id, config)
        .await
}

pub(crate) async fn reviewed_update_from_staging_repo(
    state: &AppState,
    owner: &str,
    repo_name: &str,
    staging_repo: &FsPath,
    author_id: &str,
    config: RepoConfig,
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
    let repo = find_repo(state, owner, repo_name).await?;
    let repo_id = crate::domain::store::repo_id(owner, repo_name);
    let message = pushed_commit_message(staging_repo, &head_oid)?;
    let live_tree = repo.live_tree();
    let pushed_entries = git_tree_entries(staging_repo, &head_oid)?;
    let changed_contents = git_tree_blob_contents(staging_repo, &pushed_entries)?;
    let mut changes = Vec::new();
    let mut uploaded_file_blobs = Vec::new();
    let mut pushed_paths = BTreeSet::new();
    let mut changed_paths = Vec::new();
    let mut changed_entries = Vec::new();
    let mut changed_entry_contents = Vec::new();

    for (entry, content) in pushed_entries.into_iter().zip(changed_contents) {
        let path = match repo_scope_path(&entry.path) {
            Ok(path) => path,
            Err(error) => {
                crate::state::best_effort_cleanup_rollback_source_blobs(
                    state,
                    &uploaded_file_blobs,
                )
                .await;
                return Err(error.into());
            }
        };
        pushed_paths.insert(path.clone());
        let live_content = live_tree.get(&path);
        if live_content.is_some_and(|blob| {
            blob.git_oid == entry.oid
                && blob.git_file_mode == entry.mode
                && blob.size_bytes == entry.size_bytes as u64
        }) {
            continue;
        }

        changed_paths.push((path, live_content.cloned()));
        changed_entries.push(entry);
        changed_entry_contents.push(content);
    }

    let changed_blobs = match put_git_blob_contents(
        state,
        &repo_id,
        &changed_entries,
        &changed_entry_contents,
        &mut uploaded_file_blobs,
    ) {
        Ok(blobs) => blobs,
        Err(error) => {
            crate::state::best_effort_cleanup_rollback_source_blobs(state, &uploaded_file_blobs)
                .await;
            return Err(error);
        }
    };
    for ((path, live_content), new_content) in changed_paths.into_iter().zip(changed_blobs) {
        if !source_content_matches(live_content.as_ref(), Some(&new_content)) {
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
            crate::state::best_effort_cleanup_rollback_source_blobs(state, &uploaded_file_blobs)
                .await;
            return Err(error);
        }
    };

    Ok(ReceivePackUpdate {
        branch,
        head_oid,
        base_git_snapshot_key: None,
        author_id: author_id.to_string(),
        message,
        git_snapshot,
        uploaded_blobs: uploaded_file_blobs,
        changes,
        previous_config: None,
        base_config_hash: crate::state::repo_config_fingerprint(&repo.repo_config)?,
        config,
    })
}
