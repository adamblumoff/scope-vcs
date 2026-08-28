use super::repo_io::{
    FencedGitPush, GitSegmentUploadHeartbeat, GitTreeFile, describe_refs, git_changed_tree_entries,
    git_push_from_repo, git_refs, git_tree_entries_under, pushed_commit_message,
    queue_failed_git_objects, run_git_output_bounded, validate_pushed_commit_range,
};
use super::staging::{ReceivePackFileChange, ReceivePackUpdate, ensure_default_branch};
use crate::{error::ApiError, git::content::git_blob_reference, state::AppState};
use scope_domain::landing_file::{
    MAX_REPOSITORY_LANDING_FILE_BYTES, REPOSITORY_LANDING_FILE_PATH, RepositoryLandingFile,
    RepositoryLandingFileMutation,
};
use scope_domain::policy::ScopePath;
use scope_domain::repo_config::RepoConfig;
use scope_domain::repository::RepoLifecycleState;
use scope_domain::runs::{
    catalog::{
        MAX_REPOSITORY_WORKFLOW_FILES, MAX_WORKFLOW_DEFINITION_BYTES, RepositoryWorkflowCatalog,
        RepositoryWorkflowFile,
    },
    workflow::identity::WorkflowPath,
};
use scope_git_storage::StagedGitSegment;
use scope_postgres::db::ContentRefFence;
use scope_postgres::db::RepositoryGitWriteLease;
use std::{path::Path as FsPath, time::Instant};

#[derive(Clone, Copy, PartialEq, Eq)]
enum ReviewedUpdateMode {
    FirstPush,
    ReadyPush,
    RequestMerge,
}

pub(crate) struct PreparedReceivePackUpdate {
    pub(crate) update: ReceivePackUpdate,
    pub(crate) fence: ContentRefFence,
    pub(crate) staged_segment: StagedGitSegment,
    pub(crate) write_lease: RepositoryGitWriteLease,
    pub(crate) upload_heartbeat: GitSegmentUploadHeartbeat,
}

impl std::fmt::Debug for PreparedReceivePackUpdate {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("PreparedReceivePackUpdate")
            .field("update", &self.update)
            .finish_non_exhaustive()
    }
}

impl std::ops::Deref for PreparedReceivePackUpdate {
    type Target = ReceivePackUpdate;

    fn deref(&self) -> &Self::Target {
        &self.update
    }
}

impl std::ops::DerefMut for PreparedReceivePackUpdate {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.update
    }
}

pub(crate) async fn receive_pack_update_from_staging_repo(
    state: &AppState,
    owner: &str,
    repo_name: &str,
    staging_repo: &FsPath,
    author_id: &str,
    config: RepoConfig,
) -> Result<PreparedReceivePackUpdate, ApiError> {
    reviewed_update_from_staging_repo_mode(
        state,
        owner,
        repo_name,
        staging_repo,
        author_id,
        config,
        ReviewedUpdateMode::ReadyPush,
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
) -> Result<PreparedReceivePackUpdate, ApiError> {
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
) -> Result<PreparedReceivePackUpdate, ApiError> {
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
) -> Result<PreparedReceivePackUpdate, ApiError> {
    let refs = git_refs(staging_repo)?;
    if refs.len() != 1 {
        return Err(ApiError::bad_request(format!(
            "push must update exactly one branch and no tags; found {}",
            describe_refs(&refs)
        )));
    }
    let (branch, head_oid) = refs.into_iter().next().expect("length checked");
    ensure_default_branch(&branch)?;
    let repository_id = scope_domain::repository::repo_id(owner, repo_name);
    let write_lease = state
        .metadata
        .repositories()
        .acquire_git_write_lease(&repository_id)
        .await?;
    let repo = state
        .metadata
        .repositories()
        .git_push_context(owner, repo_name, author_id)
        .await?
        .ok_or_else(|| ApiError::not_found(format!("repo {owner}/{repo_name} not found")))?;
    if mode != ReviewedUpdateMode::FirstPush && repo.lifecycle_state != RepoLifecycleState::Ready {
        return Err(ApiError::conflict("repo must be ready before push"));
    }
    let base_config_hash = crate::push_intents::repo_config_fingerprint(&repo.repo_config)?;
    let message = pushed_commit_message(staging_repo, &head_oid)?;
    let base_head_oid = repo.git_head.as_ref().map(|head| head.head_oid.as_str());
    validate_pushed_commit_range(staging_repo, base_head_oid, &head_oid)?;
    let diff_started = Instant::now();
    let pushed_entries = git_changed_tree_entries(staging_repo, base_head_oid, &head_oid)?;
    let diff_ms = diff_started.elapsed().as_millis();
    if pushed_entries.is_empty() && mode != ReviewedUpdateMode::RequestMerge {
        return Err(ApiError::bad_request(
            "receive-pack update did not change the live tree",
        ));
    }
    let pack_started = Instant::now();
    let FencedGitPush {
        stored: mut created_push,
        fence,
        staged_segment,
        upload_heartbeat,
    } = match git_push_from_repo(state, &repo.repo_id, staging_repo, repo.git_head.as_ref()).await {
        Ok(snapshot) => snapshot,
        Err(error) => {
            return Err(error);
        }
    };
    created_push.head.change_version = repo.change_version.saturating_add(1);
    let pack_put_ms = pack_started.elapsed().as_millis();
    let pack_bytes = created_push.pack_span.segment.plaintext_bytes;
    let durable_objects = vec![created_push.head.manifest.clone()];
    let landing_file_mutation = match repository_landing_file_mutation(
        staging_repo,
        &pushed_entries,
        &created_push.head.manifest,
    ) {
        Ok(mutation) => mutation,
        Err(error) => {
            cleanup_prepared_git_push(state, &repo.repo_id, &staged_segment, durable_objects)
                .await?;
            return Err(error);
        }
    };
    let changes = match pushed_entries
        .into_iter()
        .map(|(path, entry)| {
            Ok(ReceivePackFileChange {
                path,
                content: entry
                    .map(|entry| {
                        git_blob_reference(
                            &created_push.head.manifest,
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
            cleanup_prepared_git_push(state, &repo.repo_id, &staged_segment, durable_objects)
                .await?;
            return Err(error);
        }
    };

    tracing::info!(
        owner,
        repo = repo_name,
        changed_files = changes.len(),
        pack_bytes,
        diff_ms,
        pack_put_ms,
        "prepared durable Git push objects"
    );

    let workflow_catalog = match capture_repository_workflow_catalog(
        staging_repo,
        &repo.repo_id,
        &head_oid,
        created_push.head.change_version,
        &created_push.head.manifest,
    ) {
        Ok(catalog) => catalog,
        Err(error) => {
            cleanup_prepared_git_push(state, &repo.repo_id, &staged_segment, durable_objects)
                .await?;
            return Err(error);
        }
    };
    Ok(PreparedReceivePackUpdate {
        update: ReceivePackUpdate {
            branch,
            head_oid,
            base_git_manifest_ref: None,
            author_id: author_id.to_string(),
            message,
            git_head: created_push.head,
            git_pack_span: created_push.pack_span,
            durable_objects,
            workflow_catalog,
            landing_file_mutation,
            changes,
            previous_config: Some(repo.repo_config.clone()),
            base_config_hash,
            config,
        },
        fence,
        staged_segment,
        upload_heartbeat,
        write_lease,
    })
}

async fn cleanup_prepared_git_push(
    state: &AppState,
    repository_id: &str,
    staged_segment: &StagedGitSegment,
    durable_objects: Vec<scope_domain::content::SourceBlob>,
) -> Result<(), ApiError> {
    super::repo_io::best_effort_delete_staged_git_segment(state, repository_id, staged_segment)
        .await;
    queue_failed_git_objects(state, durable_objects).await
}

fn repository_landing_file_mutation(
    staging_repo: &FsPath,
    pushed_entries: &[(ScopePath, Option<GitTreeFile>)],
    git_manifest: &scope_domain::content::SourceBlob,
) -> Result<RepositoryLandingFileMutation, ApiError> {
    let Some((_, entry)) = pushed_entries
        .iter()
        .find(|(path, _)| path.as_str() == REPOSITORY_LANDING_FILE_PATH)
    else {
        return Ok(RepositoryLandingFileMutation::Unchanged);
    };
    let Some(entry) = entry else {
        return Ok(RepositoryLandingFileMutation::Delete);
    };
    if entry.size_bytes > MAX_REPOSITORY_LANDING_FILE_BYTES {
        return Ok(RepositoryLandingFileMutation::Delete);
    }

    let source = git_blob_reference(
        git_manifest,
        entry.oid.clone(),
        entry.mode.clone(),
        entry.size_bytes,
    )?;
    let output = run_git_output_bounded(
        Some(staging_repo),
        &["cat-file", "blob", &entry.oid],
        "reading repository landing file",
        MAX_REPOSITORY_LANDING_FILE_BYTES,
    )?;
    if !output.status.success() || output.stdout.len() != entry.size_bytes {
        return Err(ApiError::infrastructure_unavailable(
            "reading repository landing file failed",
        ));
    }
    RepositoryLandingFile::from_source_blob(&source, output.stdout)
        .map(RepositoryLandingFileMutation::Upsert)
        .map_err(ApiError::from)
}

fn capture_repository_workflow_catalog(
    staging_repo: &FsPath,
    repository_id: &str,
    head_oid: &str,
    change_version: u64,
    git_manifest: &scope_domain::content::SourceBlob,
) -> Result<RepositoryWorkflowCatalog, ApiError> {
    let workflow_entries = git_tree_entries_under(staging_repo, head_oid, ".scope/runs")?;
    if workflow_entries.len() > MAX_REPOSITORY_WORKFLOW_FILES {
        return RepositoryWorkflowCatalog::rejected(
            repository_id,
            head_oid,
            change_version,
            format!(
                "repository contains more than {MAX_REPOSITORY_WORKFLOW_FILES} workflow definitions"
            ),
        )
        .map_err(ApiError::internal);
    }

    let mut workflows = Vec::with_capacity(workflow_entries.len());
    for entry in workflow_entries {
        let path = format!("/{}", entry.path);
        if WorkflowPath::parse(path.clone()).is_err() {
            return RepositoryWorkflowCatalog::rejected(
                repository_id,
                head_oid,
                change_version,
                format!("invalid workflow path {path}"),
            )
            .map_err(ApiError::internal);
        }
        if entry.size_bytes > MAX_WORKFLOW_DEFINITION_BYTES {
            return RepositoryWorkflowCatalog::rejected(
                repository_id,
                head_oid,
                change_version,
                format!("workflow {path} exceeds {MAX_WORKFLOW_DEFINITION_BYTES} bytes"),
            )
            .map_err(ApiError::internal);
        }
        let source = git_blob_reference(
            git_manifest,
            entry.oid.clone(),
            entry.mode,
            entry.size_bytes,
        )?;
        let output = run_git_output_bounded(
            Some(staging_repo),
            &["cat-file", "blob", &entry.oid],
            "reading repository workflow definition",
            MAX_WORKFLOW_DEFINITION_BYTES,
        )?;
        if !output.status.success() || output.stdout.len() != entry.size_bytes {
            return Err(ApiError::infrastructure_unavailable(format!(
                "reading repository workflow {path} failed"
            )));
        }
        workflows.push(
            RepositoryWorkflowFile::from_source_blob(path, &source, output.stdout)
                .map_err(ApiError::internal)?,
        );
    }
    RepositoryWorkflowCatalog::captured(repository_id, head_oid, change_version, workflows)
        .map_err(ApiError::internal)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::git::import::{run_git, run_git_output, validate_pushed_file_path};
    use scope_domain::{
        content::{DEFAULT_GIT_FILE_MODE, SourceBlob},
        content_ref::ContentRef,
        policy::ScopePath,
    };
    use std::{
        fs,
        path::PathBuf,
        time::{SystemTime, UNIX_EPOCH},
    };

    #[test]
    fn landing_file_mutation_distinguishes_unchanged_delete_and_oversized() {
        let manifest = git_manifest();
        assert_eq!(
            repository_landing_file_mutation(FsPath::new("unused"), &[], &manifest).unwrap(),
            RepositoryLandingFileMutation::Unchanged
        );

        let path = ScopePath::parse(REPOSITORY_LANDING_FILE_PATH).unwrap();
        assert_eq!(
            repository_landing_file_mutation(
                FsPath::new("unused"),
                &[(path.clone(), None)],
                &manifest,
            )
            .unwrap(),
            RepositoryLandingFileMutation::Delete
        );

        let oversized = GitTreeFile {
            path: validate_pushed_file_path("README.html").unwrap(),
            mode: DEFAULT_GIT_FILE_MODE.to_string(),
            oid: "unused".to_string(),
            size_bytes: MAX_REPOSITORY_LANDING_FILE_BYTES + 1,
        };
        assert_eq!(
            repository_landing_file_mutation(
                FsPath::new("unused"),
                &[(path, Some(oversized))],
                &manifest,
            )
            .unwrap(),
            RepositoryLandingFileMutation::Delete
        );
    }

    #[test]
    fn landing_file_upsert_reads_the_changed_git_blob() {
        let repo = temp_repo_path("landing-file");
        run_git(
            None,
            &[
                "init",
                "--initial-branch=main",
                repo.to_string_lossy().as_ref(),
            ],
            "initializing landing file test repository",
        )
        .unwrap();
        let bytes = b"<!doctype html><h1>fast</h1>";
        fs::write(repo.join("README.html"), bytes).unwrap();
        let output = run_git_output(
            Some(&repo),
            &["hash-object", "-w", "README.html"],
            "writing landing file test blob",
        )
        .unwrap();
        assert!(output.status.success());
        let oid = String::from_utf8(output.stdout).unwrap().trim().to_string();
        let entry = GitTreeFile {
            path: validate_pushed_file_path("README.html").unwrap(),
            mode: DEFAULT_GIT_FILE_MODE.to_string(),
            oid: oid.clone(),
            size_bytes: bytes.len(),
        };

        let mutation = repository_landing_file_mutation(
            &repo,
            &[(
                ScopePath::parse(REPOSITORY_LANDING_FILE_PATH).unwrap(),
                Some(entry),
            )],
            &git_manifest(),
        )
        .unwrap();
        let RepositoryLandingFileMutation::Upsert(file) = mutation else {
            panic!("expected landing file upsert");
        };
        assert_eq!(file.oid, oid);
        assert_eq!(file.content_bytes, bytes);
        assert_eq!(file.size_bytes, bytes.len() as u64);

        fs::remove_dir_all(repo).unwrap();
    }

    fn git_manifest() -> SourceBlob {
        SourceBlob {
            content_ref: ContentRef::git_manifest_sha256("manifest"),
            sha256: "manifest".to_string(),
            git_oid: "head".to_string(),
            git_file_mode: DEFAULT_GIT_FILE_MODE.to_string(),
            size_bytes: 1,
        }
    }

    fn temp_repo_path(label: &str) -> PathBuf {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!("scope-{label}-{}-{nonce}", std::process::id()))
    }
}
