use crate::{
    auth::scope::principal_for_user_id,
    config::DEFAULT_GIT_BRANCH,
    error::ApiError,
    git::{
        import::{
            PreparedReceivePackUpdate, ReceivePackUpdate, request_merge_update_from_staging_repo,
            run_git, run_git_output,
        },
        projection_repo::verify_projection_materialization,
        request_ref_public_safety::validate_public_request_merge_range,
        request_refs::attach_visible_request_refs,
        storage::{receive_pack_staging_repo_path, remove_dir_if_exists},
    },
    persistence::{ensure_private_dir, unix_now},
    product_analytics::ProductEvent,
    repo_access::{ensure_repo_read, find_repo},
    repo_events::RepoChangeReason,
    state::AppState,
    use_cases::content_cleanup::best_effort_cleanup_rollback_source_blobs,
};
use scope_domain::{
    content::SourceBlob,
    landing_file::RepositoryLandingFileMutation,
    projection::{ProjectionViewKey, project_graph},
    repo_actions::reviewed_update_domain_error,
    repository::updates::RequestMergeOrigin,
    repository::{
        Repository,
        access::{RepositoryAccess, RepositoryActor},
    },
    requests::{
        MergeRequestInput, Request, RequestAudience, RequestViewer, canonical_request_ref,
        request_actor_role, request_policy,
    },
    reviewed_updates::content::apply_request_merge_to_repo,
    runs::catalog::RepositoryWorkflowCatalog,
};
use scope_git_storage::StagedGitSegment;
use scope_postgres::db::ContentRefFence;
use scope_postgres::db::RepositoryGitWriteLease;

pub(crate) struct MergeRequestCommand {
    pub(crate) owner: String,
    pub(crate) repo_name: String,
    pub(crate) request_id: String,
    pub(crate) actor_user_id: String,
}

pub(crate) struct MergeRequestResult {
    pub(crate) repo: Repository,
    pub(crate) access: RepositoryAccess,
    pub(crate) actor_user_id: String,
    pub(crate) request: Request,
}

struct PersistedRequestMerge {
    request: Request,
    repo_change_version: u64,
}

pub(crate) struct PreparedRequestMerge {
    pub(crate) repository_id: String,
    pub(crate) expected_manifest_ref: scope_domain::content_ref::ContentRef,
    pub(crate) expected_repo_change_version: u64,
    pub(crate) prepared_request_head_oid: String,
    pub(crate) origin: RequestMergeOrigin,
    pub(crate) landing_file_mutation: RepositoryLandingFileMutation,
    pub(crate) workflow_catalog: RepositoryWorkflowCatalog,
    pub(crate) update: ReceivePackUpdate,
    pub(crate) fence: ContentRefFence,
    pub(crate) staged_segment: StagedGitSegment,
    pub(crate) write_lease: RepositoryGitWriteLease,
}

impl PreparedRequestMerge {
    pub(crate) fn durable_objects(&self) -> &[SourceBlob] {
        &self.update.durable_objects
    }
}

pub(crate) async fn merge_request(
    state: &AppState,
    command: MergeRequestCommand,
) -> Result<MergeRequestResult, ApiError> {
    let repo = find_repo(state, &command.owner, &command.repo_name).await?;
    let principal = principal_for_user_id(&repo, &command.actor_user_id);
    ensure_repo_read(state, &repo, &principal)?;
    let access = repo.access_for_principal(&principal);
    let request = state
        .metadata
        .requests()
        .request_by_id(&command.request_id)
        .await?
        .ok_or_else(|| ApiError::not_found("request not found"))?;
    let is_invitee = state
        .metadata
        .requests()
        .request_is_invitee(&request.id, &command.actor_user_id)
        .await?;
    let policy = request_policy(
        &request,
        RequestViewer::new(access, Some(&command.actor_user_id), is_invitee),
    );
    if request.repo_id != repo.record.id || !policy.exact_visible {
        return Err(ApiError::not_found("request not found"));
    }
    if !policy.permissions.can_merge {
        if matches!(access.actor, RepositoryActor::Public) {
            return Err(ApiError::forbidden("repo maintainer required"));
        }
        return Err(ApiError::conflict("request cannot be merged"));
    }

    let analytics_event = ProductEvent::request_merged(
        &command.actor_user_id,
        request.audience,
        request_actor_role(access),
    );
    let prepared = prepare_request_merge(
        state,
        &command.owner,
        &command.repo_name,
        &command.actor_user_id,
        &repo,
        &request,
    )
    .await?;
    let merged_event_id = match random_id("event_request_merged") {
        Ok(event_id) => event_id,
        Err(error) => {
            cleanup_prepared_merge(state, prepared).await;
            return Err(error);
        }
    };
    let now_unix = match unix_now() {
        Ok(now_unix) => now_unix,
        Err(error) => {
            cleanup_prepared_merge(state, prepared).await;
            return Err(error);
        }
    };
    let mutation =
        persist_prepared_merge(state, &command, merged_event_id, now_unix, prepared).await?;

    state.product_analytics.capture(analytics_event);
    state
        .publish_repo_change(
            &repo.record.id,
            mutation.repo_change_version,
            RepoChangeReason::RequestMerged,
        )
        .await;
    let committed_repo = find_repo(state, &command.owner, &command.repo_name).await?;
    state
        .publish_request_summary_refresh(&committed_repo.record.id, RepoChangeReason::RequestMerged)
        .await;
    Ok(MergeRequestResult {
        repo: committed_repo,
        access,
        actor_user_id: command.actor_user_id,
        request: mutation.request,
    })
}

async fn persist_prepared_merge(
    state: &AppState,
    command: &MergeRequestCommand,
    merged_event_id: String,
    now_unix: u64,
    prepared: PreparedRequestMerge,
) -> Result<PersistedRequestMerge, ApiError> {
    let durable_objects = prepared.durable_objects().to_vec();
    let fence = prepared.fence;
    let staged_segment = prepared.staged_segment;
    let write_lease = prepared.write_lease;
    let repository_id = scope_domain::repository::repo_id(&command.owner, &command.repo_name);
    let mutation = state
        .metadata
        .requests()
        .merge_request_content(
            &command.owner,
            &command.repo_name,
            &prepared.expected_manifest_ref,
            prepared.expected_repo_change_version,
            &prepared.prepared_request_head_oid,
            prepared.update.into_reviewed_update(),
            prepared.landing_file_mutation,
            prepared.workflow_catalog,
            prepared.origin,
            MergeRequestInput {
                request_id: command.request_id.clone(),
                actor_user_id: command.actor_user_id.clone(),
                actor_is_maintainer: false,
                merged_head_oid: String::new(),
                merged_main_oid: String::new(),
                merged_event_id,
                now_unix,
            },
            &crate::persistence_ids::generate_persistence_id,
        )
        .await;
    match mutation {
        Ok(mutation) => {
            fence.release().await;
            if let Err(error) = state.git_segment_store.delete_local(&staged_segment).await {
                tracing::warn!(
                    repository_id,
                    segment_id = staged_segment.segment.segment_id,
                    error = %error,
                    "merged Git segment local staging cleanup failed"
                );
            }
            write_lease.release().await;
            Ok(PersistedRequestMerge {
                request: mutation.request.request,
                repo_change_version: mutation.git_head.change_version,
            })
        }
        Err(error) => {
            crate::git::import::best_effort_delete_staged_git_segment(
                state,
                &repository_id,
                &staged_segment,
            )
            .await;
            best_effort_cleanup_rollback_source_blobs(state, &durable_objects).await;
            fence.release().await;
            write_lease.release().await;
            Err(error.into())
        }
    }
}

pub(crate) async fn prepare_request_merge(
    state: &AppState,
    owner: &str,
    repo_name: &str,
    actor_user_id: &str,
    repo: &Repository,
    request: &Request,
) -> Result<PreparedRequestMerge, ApiError> {
    let current = repo
        .git_head
        .as_ref()
        .ok_or_else(|| ApiError::conflict("repo has no accepted Git head"))?;
    let base_repo = state.repository_engine.materialize_repository(
        state,
        &repo.record.id,
        current,
        &repo.git_pack_spans,
    )?;
    let staging_repo = receive_pack_staging_repo_path(state, owner, repo_name)?;
    if let Some(parent) = staging_repo.parent() {
        ensure_private_dir(parent)?;
    }
    run_git(
        None,
        &[
            "clone",
            "--bare",
            "--no-hardlinks",
            base_repo.to_string_lossy().as_ref(),
            staging_repo.to_string_lossy().as_ref(),
        ],
        "preparing request merge repository",
    )?;
    let prepared = async {
        attach_visible_request_refs(state, std::slice::from_ref(request), &staging_repo, None)?;
        let request_ref = canonical_request_ref(&request.name);
        let (origin, merge_base_oid) = match request.audience {
            RequestAudience::Public => {
                let validated = validate_public_request_merge_range(
                    repo,
                    state,
                    &staging_repo,
                    &request.head_oid,
                )?;
                let merge_base_oid = validated.public_base_oid.clone();
                (
                    RequestMergeOrigin::Public {
                        request_id: request.id.clone(),
                        public_base_oid: validated.public_base_oid,
                        public_parent_oids: validated.public_parent_oids,
                        request_head_oid: request.head_oid.clone(),
                        commits: validated.commits,
                    },
                    merge_base_oid,
                )
            }
            RequestAudience::Private => (
                RequestMergeOrigin::Private {
                    request_id: request.id.clone(),
                    request_head_oid: request.head_oid.clone(),
                },
                request.base_main_oid.clone(),
            ),
        };
        let merged_main_oid = merge_main_oid(
            &staging_repo,
            &merge_base_oid,
            &current.head_oid,
            &request.head_oid,
            &request.name,
        )?;
        let main_ref = format!("refs/heads/{DEFAULT_GIT_BRANCH}");
        run_git(
            Some(&staging_repo),
            &["update-ref", &main_ref, &merged_main_oid],
            "updating prepared merge main",
        )?;
        run_git(
            Some(&staging_repo),
            &["update-ref", "-d", &request_ref],
            "removing prepared request branch",
        )?;
        let PreparedReceivePackUpdate {
            update,
            fence,
            staged_segment,
            write_lease,
        } = request_merge_update_from_staging_repo(
            state,
            owner,
            repo_name,
            &staging_repo,
            actor_user_id,
            repo.repo_config.clone(),
        )
        .await?;
        let preflight = (|| -> Result<(), ApiError> {
            let mut proposed_repo = repo.clone();
            apply_request_merge_to_repo(
                &mut proposed_repo,
                update.clone().into_reviewed_update(),
                origin.clone(),
            )
            .map_err(reviewed_update_domain_error)
            .map_err(ApiError::from)?;
            let public_projection = project_graph(
                &proposed_repo.graph,
                &proposed_repo.visibility_change_sets,
                ProjectionViewKey::Public,
            );
            verify_projection_materialization(
                state,
                &public_projection,
                &staging_repo,
                &update.git_head.manifest,
            )
        })();
        if let Err(error) = preflight {
            crate::git::import::best_effort_delete_staged_git_segment(
                state,
                &repo.record.id,
                &staged_segment,
            )
            .await;
            best_effort_cleanup_rollback_source_blobs(state, &update.durable_objects).await;
            fence.release().await;
            write_lease.release().await;
            return Err(error);
        }
        Ok(PreparedRequestMerge {
            repository_id: repo.record.id.clone(),
            expected_manifest_ref: current.manifest.content_ref.clone(),
            expected_repo_change_version: repo.record.change_version,
            prepared_request_head_oid: request.head_oid.clone(),
            origin,
            landing_file_mutation: update.landing_file_mutation.clone(),
            workflow_catalog: update.workflow_catalog.clone(),
            update,
            fence,
            staged_segment,
            write_lease,
        })
    }
    .await;
    let cleanup = remove_dir_if_exists(&staging_repo);
    match (prepared, cleanup) {
        (Ok(value), Ok(())) => Ok(value),
        (Err(error), _) => Err(error),
        (Ok(value), Err(error)) => {
            crate::git::import::best_effort_delete_staged_git_segment(
                state,
                &scope_domain::repository::repo_id(owner, repo_name),
                &value.staged_segment,
            )
            .await;
            best_effort_cleanup_rollback_source_blobs(state, &value.update.durable_objects).await;
            value.fence.release().await;
            value.write_lease.release().await;
            Err(error)
        }
    }
}

async fn cleanup_prepared_merge(state: &AppState, prepared: PreparedRequestMerge) {
    crate::git::import::best_effort_delete_staged_git_segment(
        state,
        &prepared.repository_id,
        &prepared.staged_segment,
    )
    .await;
    best_effort_cleanup_rollback_source_blobs(state, prepared.durable_objects()).await;
    prepared.fence.release().await;
    prepared.write_lease.release().await;
}

#[cfg(test)]
pub(crate) async fn persist_prepared_merge_for_tests(
    state: &AppState,
    owner: &str,
    repo_name: &str,
    request_id: &str,
    actor_user_id: &str,
    prepared: PreparedRequestMerge,
) -> Result<Request, ApiError> {
    let command = MergeRequestCommand {
        owner: owner.to_string(),
        repo_name: repo_name.to_string(),
        request_id: request_id.to_string(),
        actor_user_id: actor_user_id.to_string(),
    };
    persist_prepared_merge(
        state,
        &command,
        "event_request_merged_test".to_string(),
        10,
        prepared,
    )
    .await
    .map(|mutation| mutation.request)
}

fn random_id(prefix: &str) -> Result<String, ApiError> {
    let mut bytes = [0_u8; 16];
    getrandom::fill(&mut bytes).map_err(|error| {
        ApiError::internal_message(format!("failed to create {prefix} id: {error}"))
    })?;
    Ok(format!("{prefix}_{}", hex::encode(bytes)))
}

fn merge_main_oid(
    repo: &std::path::Path,
    request_base_oid: &str,
    current_main_oid: &str,
    request_head_oid: &str,
    request_name: &str,
) -> Result<String, ApiError> {
    run_git(
        Some(repo),
        &["config", "user.name", "Scope"],
        "configuring request merge author",
    )?;
    run_git(
        Some(repo),
        &["config", "user.email", "merge@scope.local"],
        "configuring request merge email",
    )?;
    let synthetic_base =
        synthetic_merge_commit(repo, request_base_oid, None, "Scope request merge base")?;
    let synthetic_main = synthetic_merge_commit(
        repo,
        current_main_oid,
        Some(&synthetic_base),
        "Scope current main",
    )?;
    let synthetic_request = synthetic_merge_commit(
        repo,
        request_head_oid,
        Some(&synthetic_base),
        "Scope request head",
    )?;
    let merge_tree = run_git_output(
        Some(repo),
        &[
            "merge-tree",
            "--write-tree",
            &synthetic_main,
            &synthetic_request,
        ],
        "merging request trees",
    )?;
    if !merge_tree.status.success() {
        return Err(ApiError::conflict(format!(
            "request cannot merge cleanly: {}",
            String::from_utf8_lossy(&merge_tree.stderr).trim()
        )));
    }
    let tree_oid = String::from_utf8(merge_tree.stdout)
        .map_err(ApiError::internal)?
        .lines()
        .next()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| ApiError::internal_message("Git merge-tree returned no tree"))?
        .to_string();
    let message = format!("Merge request {request_name}");
    let commit = run_git_output(
        Some(repo),
        &[
            "commit-tree",
            &tree_oid,
            "-p",
            current_main_oid,
            "-p",
            request_head_oid,
            "-m",
            &message,
        ],
        "creating request merge commit",
    )?;
    if !commit.status.success() {
        return Err(ApiError::infrastructure_unavailable(format!(
            "creating request merge commit: {}",
            String::from_utf8_lossy(&commit.stderr).trim()
        )));
    }
    String::from_utf8(commit.stdout)
        .map_err(ApiError::internal)
        .map(|value| value.trim().to_string())
}

fn synthetic_merge_commit(
    repo: &std::path::Path,
    tree_source_oid: &str,
    parent_oid: Option<&str>,
    message: &str,
) -> Result<String, ApiError> {
    let tree_source = format!("{tree_source_oid}^{{tree}}");
    let mut args = vec!["commit-tree", tree_source.as_str()];
    if let Some(parent_oid) = parent_oid {
        args.extend(["-p", parent_oid]);
    }
    args.extend(["-m", message]);
    let commit = run_git_output(Some(repo), &args, "creating synthetic request merge commit")?;
    if !commit.status.success() {
        return Err(ApiError::infrastructure_unavailable(format!(
            "creating synthetic request merge commit: {}",
            String::from_utf8_lossy(&commit.stderr).trim()
        )));
    }
    String::from_utf8(commit.stdout)
        .map_err(ApiError::internal)
        .map(|value| value.trim().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        fs,
        path::{Path, PathBuf},
        time::{SystemTime, UNIX_EPOCH},
    };

    #[test]
    fn explicit_request_base_preserves_private_main_files() {
        let repo = temp_repo_path("preserves-private");
        run_git(
            None,
            &[
                "init",
                "--initial-branch=main",
                repo.to_string_lossy().as_ref(),
            ],
            "initializing merge test repository",
        )
        .unwrap();
        run_git(
            Some(&repo),
            &["config", "user.name", "Test"],
            "configuring test name",
        )
        .unwrap();
        run_git(
            Some(&repo),
            &["config", "user.email", "test@scope.local"],
            "configuring test email",
        )
        .unwrap();

        fs::write(repo.join("public.txt"), "public base\n").unwrap();
        commit_all(&repo, "public base");
        let request_base = oid(&repo, "HEAD");

        fs::write(repo.join("private.txt"), "private main\n").unwrap();
        commit_all(&repo, "private main change");
        let current_main = oid(&repo, "HEAD");

        run_git(
            Some(&repo),
            &["switch", "--create", "request", &request_base],
            "creating request branch",
        )
        .unwrap();
        fs::write(repo.join("public.txt"), "public request\n").unwrap();
        commit_all(&repo, "request change");
        let request_head = oid(&repo, "HEAD");

        let merged = merge_main_oid(
            &repo,
            &request_base,
            &current_main,
            &request_head,
            "public-request",
        )
        .unwrap();
        assert_eq!(
            git_text(&repo, &["show", &format!("{merged}:private.txt")]),
            "private main\n"
        );
        assert_eq!(
            git_text(&repo, &["show", &format!("{merged}:public.txt")]),
            "public request\n"
        );
        let parents = git_text(&repo, &["show", "-s", "--format=%P", &merged]);
        assert_eq!(
            parents.split_ascii_whitespace().collect::<Vec<_>>(),
            [current_main.as_str(), request_head.as_str()]
        );
        let _ = fs::remove_dir_all(repo);
    }

    #[test]
    fn explicit_request_base_merges_non_overlapping_file_edits() {
        let repo = temp_repo_path("content-merge");
        run_git(
            None,
            &[
                "init",
                "--initial-branch=main",
                repo.to_string_lossy().as_ref(),
            ],
            "initializing merge test repository",
        )
        .unwrap();
        run_git(
            Some(&repo),
            &["config", "user.name", "Test"],
            "configuring test name",
        )
        .unwrap();
        run_git(
            Some(&repo),
            &["config", "user.email", "test@scope.local"],
            "configuring test email",
        )
        .unwrap();

        fs::write(repo.join("shared.txt"), "top\nmiddle\nbottom\n").unwrap();
        commit_all(&repo, "shared base");
        let request_base = oid(&repo, "HEAD");

        fs::write(repo.join("shared.txt"), "main top\nmiddle\nbottom\n").unwrap();
        commit_all(&repo, "main edit");
        let current_main = oid(&repo, "HEAD");

        run_git(
            Some(&repo),
            &["switch", "--create", "request", &request_base],
            "creating request branch",
        )
        .unwrap();
        fs::write(repo.join("shared.txt"), "top\nmiddle\nrequest bottom\n").unwrap();
        commit_all(&repo, "request edit");
        let request_head = oid(&repo, "HEAD");

        let merged = merge_main_oid(
            &repo,
            &request_base,
            &current_main,
            &request_head,
            "content-merge",
        )
        .unwrap();
        assert_eq!(
            git_text(&repo, &["show", &format!("{merged}:shared.txt")]),
            "main top\nmiddle\nrequest bottom\n"
        );
        let _ = fs::remove_dir_all(repo);
    }

    fn commit_all(repo: &Path, message: &str) {
        run_git(Some(repo), &["add", "."], "staging merge test files").unwrap();
        run_git(
            Some(repo),
            &["commit", "-m", message],
            "committing merge test files",
        )
        .unwrap();
    }

    fn oid(repo: &Path, revision: &str) -> String {
        git_text(repo, &["rev-parse", revision]).trim().to_string()
    }

    fn git_text(repo: &Path, args: &[&str]) -> String {
        let output = run_git_output(Some(repo), args, "reading merge test repository").unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8(output.stdout).unwrap()
    }

    fn temp_repo_path(label: &str) -> PathBuf {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "scope-vcs-request-merge-{label}-{}-{nonce}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&path);
        path
    }
}
