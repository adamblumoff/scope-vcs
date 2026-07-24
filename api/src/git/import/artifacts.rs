use super::repo_io::{
    describe_refs, git_changed_tree_entries, git_refs, git_segment_manifest_from_repo,
    pushed_commit_message,
};
use super::staging::{ReceivePackFileChange, ReceivePackUpdate, ensure_default_branch};
use crate::domain::repo_config::RepoConfig;
use crate::domain::store::RepoPublicationState;
use crate::{error::ApiError, git::content::git_blob_reference, state::AppState};
use std::{path::Path as FsPath, time::Instant};

#[derive(Clone, Copy, PartialEq, Eq)]
enum ReviewedUpdateMode {
    FirstPush,
    PublishedPush,
    RequestMerge,
}

pub(crate) async fn receive_pack_update_from_staging_repo(
    state: &AppState,
    owner: &str,
    repo_name: &str,
    staging_repo: &FsPath,
    author_id: &str,
    config: RepoConfig,
) -> Result<ReceivePackUpdate, ApiError> {
    reviewed_update_from_staging_repo_mode(
        state,
        owner,
        repo_name,
        staging_repo,
        author_id,
        config,
        ReviewedUpdateMode::PublishedPush,
    )
    .await
}

pub(crate) async fn request_merge_update_from_staging_repo(
    state: &AppState,
    owner: &str,
    repo_name: &str,
    staging_repo: &FsPath,
    author_id: &str,
    config: RepoConfig,
) -> Result<ReceivePackUpdate, ApiError> {
    reviewed_update_from_staging_repo_mode(
        state,
        owner,
        repo_name,
        staging_repo,
        author_id,
        config,
        ReviewedUpdateMode::RequestMerge,
    )
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
    reviewed_update_from_staging_repo_mode(
        state,
        owner,
        repo_name,
        staging_repo,
        author_id,
        config,
        ReviewedUpdateMode::FirstPush,
    )
    .await
}

async fn reviewed_update_from_staging_repo_mode(
    state: &AppState,
    owner: &str,
    repo_name: &str,
    staging_repo: &FsPath,
    author_id: &str,
    config: RepoConfig,
    mode: ReviewedUpdateMode,
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
    let repo = state
        .metadata
        .git_push_context(owner, repo_name, author_id)
        .await?
        .ok_or_else(|| ApiError::not_found(format!("repo {owner}/{repo_name} not found")))?;
    if mode != ReviewedUpdateMode::FirstPush
        && repo.publication_state != RepoPublicationState::Published
    {
        return Err(ApiError::conflict("repo must be published before push"));
    }
    let repo_id = crate::domain::store::repo_id(owner, repo_name);
    let message = pushed_commit_message(staging_repo, &head_oid)?;
    let base_head_oid = repo.git_head.as_ref().map(|head| head.head_oid.as_str());
    let diff_started = Instant::now();
    let pushed_entries = git_changed_tree_entries(staging_repo, base_head_oid, &head_oid)?;
    let diff_ms = diff_started.elapsed().as_millis();
    let mut changes = Vec::new();
    if pushed_entries.is_empty() && mode != ReviewedUpdateMode::RequestMerge {
        return Err(ApiError::bad_request(
            "receive-pack update did not change the live tree",
        ));
    }
    let segment_started = Instant::now();
    let mut created_segment =
        match git_segment_manifest_from_repo(state, &repo_id, staging_repo, repo.git_head.as_ref())
            .await
        {
            Ok(snapshot) => snapshot,
            Err(error) => {
                return Err(error);
            }
        };
    created_segment.head.change_version = repo.change_version.saturating_add(1);
    let segment_put_ms = segment_started.elapsed().as_millis();
    let segment_bytes = created_segment.segment.object.size_bytes;
    for (path, entry) in pushed_entries {
        changes.push(ReceivePackFileChange {
            path,
            content: entry
                .map(|entry| {
                    git_blob_reference(
                        &created_segment.head.manifest,
                        entry.oid,
                        entry.mode,
                        entry.size_bytes,
                    )
                })
                .transpose()?,
        });
    }

    tracing::info!(
        owner,
        repo = repo_name,
        changed_files = changes.len(),
        segment_bytes,
        diff_ms,
        segment_put_ms,
        "prepared durable Git segment"
    );

    let durable_objects = vec![
        created_segment.segment.object.clone(),
        created_segment.head.manifest.clone(),
    ];
    Ok(ReceivePackUpdate {
        branch,
        head_oid,
        base_git_manifest_key: None,
        author_id: author_id.to_string(),
        message,
        git_head: created_segment.head,
        git_segment: created_segment.segment,
        durable_objects,
        changes,
        previous_config: Some(repo.repo_config.clone()),
        base_config_hash: crate::state::repo_config_fingerprint(&repo.repo_config)?,
        config,
    })
}
