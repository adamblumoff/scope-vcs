use crate::{
    config::DEFAULT_GIT_BRANCH,
    error::ApiError,
    git::{
        import::{
            ReceivePackUpdate, request_merge_update_from_staging_repo, run_git, run_git_output,
        },
        projection_repo::verify_projection_materialization,
        request_ref_public_safety::validate_public_request_merge_range,
        request_refs::attach_visible_request_refs,
        storage::{cached_raw_git_repo, receive_pack_staging_repo_path, remove_dir_if_exists},
    },
    persistence::ensure_private_dir,
    repo_cleanup::best_effort_cleanup_rollback_source_blobs,
    state::AppState,
};
use scope_domain::{
    projection::{ProjectionViewKey, project_graph},
    repo_actions::reviewed_update_domain_error,
    requests::{Request, RequestAudience, canonical_request_ref},
    reviewed_updates::apply_request_merge_to_repo,
    store::{RequestMergeOrigin, SourceBlob, StoredRepository},
};

pub(crate) struct PreparedRequestMerge {
    pub(crate) expected_manifest_ref: scope_domain::content_ref::ContentRef,
    pub(crate) expected_repo_change_version: u64,
    pub(crate) prepared_request_head_oid: String,
    pub(crate) origin: RequestMergeOrigin,
    pub(crate) update: ReceivePackUpdate,
}

impl PreparedRequestMerge {
    pub(crate) fn durable_objects(&self) -> &[SourceBlob] {
        &self.update.durable_objects
    }
}

pub(crate) async fn prepare_request_merge(
    state: &AppState,
    owner: &str,
    repo_name: &str,
    actor_user_id: &str,
    repo: &StoredRepository,
    request: &Request,
) -> Result<PreparedRequestMerge, ApiError> {
    let current = repo
        .git_head
        .as_ref()
        .ok_or_else(|| ApiError::conflict("repo has no accepted Git head"))?;
    let base_repo = cached_raw_git_repo(state, &current.manifest)?;
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
        let update = request_merge_update_from_staging_repo(
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
                &proposed_repo.visibility_events,
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
            best_effort_cleanup_rollback_source_blobs(state, &update.durable_objects).await;
            return Err(error);
        }
        Ok(PreparedRequestMerge {
            expected_manifest_ref: current.manifest.content_ref.clone(),
            expected_repo_change_version: repo.record.change_version,
            prepared_request_head_oid: request.head_oid.clone(),
            origin,
            update,
        })
    }
    .await;
    let cleanup = remove_dir_if_exists(&staging_repo);
    match (prepared, cleanup) {
        (Ok(value), Ok(())) => Ok(value),
        (Err(error), _) => Err(error),
        (Ok(value), Err(error)) => {
            best_effort_cleanup_rollback_source_blobs(state, &value.update.durable_objects).await;
            Err(error)
        }
    }
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
    let merge_tree = run_git_output(
        Some(repo),
        &[
            "merge-tree",
            "--write-tree",
            &format!("--merge-base={request_base_oid}"),
            current_main_oid,
            request_head_oid,
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
        return Err(ApiError::service_unavailable(format!(
            "creating request merge commit: {}",
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
