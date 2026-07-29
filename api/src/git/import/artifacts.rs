use super::repo_io::{
    describe_refs, git_changed_tree_entries, git_refs, git_segment_manifest_from_repo,
    git_snapshot_from_ref, git_tree_entries, pushed_commit_message, queue_failed_segments,
    run_git_output,
};
use super::staging::{ReceivePackFileChange, ReceivePackUpdate, ensure_default_branch};
use crate::{error::ApiError, git::content::git_blob_reference, state::AppState};
use scope_domain::repo_config::RepoConfig;
use scope_domain::runs::{
    trigger::{PushTriggerInput, PushWorkflowFile},
    workflow::WorkflowPath,
};
use scope_domain::store::RepoPublicationState;
use scope_git::DEFAULT_GIT_BRANCH;
use std::{path::Path as FsPath, time::Instant};

const MAX_PUSH_WORKFLOW_FILES: usize = 64;

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
        .repositories()
        .git_push_context(owner, repo_name, author_id)
        .await?
        .ok_or_else(|| ApiError::not_found(format!("repo {owner}/{repo_name} not found")))?;
    if mode != ReviewedUpdateMode::FirstPush
        && repo.publication_state != RepoPublicationState::Published
    {
        return Err(ApiError::conflict("repo must be published before push"));
    }
    let message = pushed_commit_message(staging_repo, &head_oid)?;
    let base_head_oid = repo.git_head.as_ref().map(|head| head.head_oid.as_str());
    let diff_started = Instant::now();
    let pushed_entries = git_changed_tree_entries(staging_repo, base_head_oid, &head_oid)?;
    let diff_ms = diff_started.elapsed().as_millis();
    if pushed_entries.is_empty() && mode != ReviewedUpdateMode::RequestMerge {
        return Err(ApiError::bad_request(
            "receive-pack update did not change the live tree",
        ));
    }
    let segment_started = Instant::now();
    let mut created_segment =
        match git_segment_manifest_from_repo(state, staging_repo, repo.git_head.as_ref()).await {
            Ok(snapshot) => snapshot,
            Err(error) => {
                return Err(error);
            }
        };
    created_segment.head.change_version = repo.change_version.saturating_add(1);
    let segment_put_ms = segment_started.elapsed().as_millis();
    let segment_bytes = created_segment.segment.object.size_bytes;
    let mut durable_objects = vec![
        created_segment.segment.object.clone(),
        created_segment.head.manifest.clone(),
    ];
    let changes = match pushed_entries
        .into_iter()
        .map(|(path, entry)| {
            Ok(ReceivePackFileChange {
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
            })
        })
        .collect::<Result<Vec<_>, ApiError>>()
    {
        Ok(changes) => changes,
        Err(error) => {
            queue_failed_segments(state, durable_objects).await?;
            return Err(error);
        }
    };

    tracing::info!(
        owner,
        repo = repo_name,
        changed_files = changes.len(),
        segment_bytes,
        diff_ms,
        segment_put_ms,
        "prepared durable Git segment"
    );

    let push_trigger_input = match prepare_push_trigger_input(
        state,
        staging_repo,
        &head_oid,
        mode == ReviewedUpdateMode::RequestMerge,
        &mut durable_objects,
    ) {
        Ok(input) => input,
        Err(error) => {
            queue_failed_segments(state, durable_objects).await?;
            return Err(error);
        }
    };
    Ok(ReceivePackUpdate {
        branch,
        head_oid,
        base_git_manifest_ref: None,
        author_id: author_id.to_string(),
        message,
        git_head: created_segment.head,
        git_segment: created_segment.segment,
        durable_objects,
        push_trigger_input,
        changes,
        previous_config: Some(repo.repo_config.clone()),
        base_config_hash: crate::push_intents::repo_config_fingerprint(&repo.repo_config)?,
        config,
    })
}

fn prepare_push_trigger_input(
    state: &AppState,
    staging_repo: &FsPath,
    head_oid: &str,
    skip: bool,
    durable_objects: &mut Vec<scope_domain::store::SourceBlob>,
) -> Result<Option<PushTriggerInput>, ApiError> {
    if skip {
        return Ok(None);
    }
    let entries = git_tree_entries(staging_repo, head_oid)?;
    let workflow_entries = entries
        .into_iter()
        .filter(|entry| entry.path.starts_with(".scope/runs/"))
        .collect::<Vec<_>>();
    let mut configuration_error = None;
    let mut workflows = Vec::new();
    if workflow_entries.len() > MAX_PUSH_WORKFLOW_FILES {
        configuration_error = Some(format!(
            "repository contains more than {MAX_PUSH_WORKFLOW_FILES} workflow definitions"
        ));
    } else {
        for entry in workflow_entries {
            let path = format!("/{}", entry.path);
            if WorkflowPath::parse(path.clone()).is_err() {
                configuration_error = Some(format!("invalid workflow path {path}"));
                break;
            }
            if entry.size_bytes > scope_run_config::MAX_WORKFLOW_DEFINITION_BYTES {
                configuration_error = Some(format!(
                    "workflow {path} exceeds {} bytes",
                    scope_run_config::MAX_WORKFLOW_DEFINITION_BYTES
                ));
                break;
            }
            let output = run_git_output(
                Some(staging_repo),
                &["cat-file", "blob", &entry.oid],
                "reading push workflow definition",
            )?;
            if !output.status.success() || output.stdout.len() != entry.size_bytes {
                return Err(ApiError::service_unavailable(format!(
                    "reading push workflow {path} failed"
                )));
            }
            workflows
                .push(PushWorkflowFile::new(path, output.stdout).map_err(ApiError::bad_request)?);
        }
    }
    let snapshot = git_snapshot_from_ref(
        state,
        staging_repo,
        &format!("refs/heads/{DEFAULT_GIT_BRANCH}"),
    )?;
    durable_objects.push(snapshot.clone());
    PushTriggerInput::new(
        head_oid.to_string(),
        snapshot,
        workflows,
        configuration_error,
    )
    .map(Some)
    .map_err(ApiError::bad_request)
}
