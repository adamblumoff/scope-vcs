use super::repo_io::{
    describe_refs, git_refs, git_snapshot_from_repo, git_stdout_text, git_tree_blob_contents,
    git_tree_entries, git_tree_files, pushed_commit_message, put_git_blob_contents,
};
use super::staging::{ReceivePackFileChange, ReceivePackUpdate, ensure_default_branch};
use crate::domain::projection_views::pending_scope_path;
use crate::domain::staged_updates::source_content_matches;
use crate::domain::store::{PendingImport, RepoPublicationState};
use crate::{
    error::ApiError,
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
    let mut changed_paths = Vec::new();
    let mut changed_entries = Vec::new();

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

        changed_paths.push((path, live_content.cloned()));
        changed_entries.push(entry);
    }

    let changed_contents = match git_tree_blob_contents(staging_repo, &changed_entries) {
        Ok(contents) => contents,
        Err(error) => {
            crate::state::best_effort_cleanup_rollback_source_blobs(state, &uploaded_file_blobs);
            return Err(error);
        }
    };
    let changed_blobs =
        match put_git_blob_contents(state, &repo_id, &changed_contents, &mut uploaded_file_blobs) {
            Ok(blobs) => blobs,
            Err(error) => {
                crate::state::best_effort_cleanup_rollback_source_blobs(
                    state,
                    &uploaded_file_blobs,
                );
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
