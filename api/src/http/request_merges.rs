use crate::{
    domain::{
        policy::{ScopePath, Visibility},
        projection::{ProjectionViewKey, project_graph},
        requests::{Request, RequestAudience, canonical_request_ref},
        store::StoredRepository,
    },
    error::ApiError,
    git::{
        cache::GitRepoHandle,
        import::{
            ReceivePackUpdate, receive_pack_update_from_staging_repo, run_git, run_git_output,
        },
        storage::{cached_raw_git_repo, receive_pack_staging_repo_path},
        upload::projection_bare_repo_for_state,
    },
    object_store::source_blob_bytes,
    persistence::ensure_private_dir,
    state::AppState,
};
use std::{fs, path::Path as FsPath};

pub(crate) async fn clean_merge_update(
    state: &AppState,
    owner: &str,
    repo_name: &str,
    repo: &StoredRepository,
    request: &Request,
    maintainer_id: &str,
    current_main_oid: &str,
) -> Result<ReceivePackUpdate, ApiError> {
    if request.audience == RequestAudience::Public {
        return clean_public_request_merge_update(
            state,
            owner,
            repo_name,
            repo,
            request,
            maintainer_id,
            current_main_oid,
        )
        .await;
    }
    clean_private_request_merge_update(
        state,
        owner,
        repo_name,
        repo,
        request,
        maintainer_id,
        current_main_oid,
    )
    .await
}

async fn clean_private_request_merge_update(
    state: &AppState,
    owner: &str,
    repo_name: &str,
    repo: &StoredRepository,
    request: &Request,
    maintainer_id: &str,
    current_main_oid: &str,
) -> Result<ReceivePackUpdate, ApiError> {
    let request_snapshot = request
        .git_snapshot
        .as_ref()
        .ok_or_else(|| ApiError::conflict("request branch has not been pushed"))?;
    let seed_repo = private_merge_seed_repo(state, repo)?;
    let work_repo = receive_pack_staging_repo_path(state, owner, repo_name)?;
    if work_repo.exists() {
        fs::remove_dir_all(&work_repo).map_err(ApiError::internal)?;
    }
    let result = async {
        ensure_merge_work_parent(&work_repo)?;
        run_git(
            None,
            &[
                "clone",
                "--no-hardlinks",
                seed_repo.to_string_lossy().as_ref(),
                work_repo.to_string_lossy().as_ref(),
            ],
            "cloning request merge worktree",
        )?;
        ensure_worktree_head(&work_repo, current_main_oid)?;
        fetch_and_merge_request_branch(state, &work_repo, request, request_snapshot)?;
        receive_pack_update_from_staging_repo(
            state,
            owner,
            repo_name,
            &work_repo,
            maintainer_id,
            repo.repo_config.clone(),
        )
        .await
    }
    .await;
    let _ = fs::remove_dir_all(&work_repo);
    result
}

async fn clean_public_request_merge_update(
    state: &AppState,
    owner: &str,
    repo_name: &str,
    repo: &StoredRepository,
    request: &Request,
    maintainer_id: &str,
    current_main_oid: &str,
) -> Result<ReceivePackUpdate, ApiError> {
    let request_snapshot = request
        .git_snapshot
        .as_ref()
        .ok_or_else(|| ApiError::conflict("request branch has not been pushed"))?;
    let public_seed_repo = public_merge_seed_repo(state, repo)?;
    let raw_seed_repo = private_merge_seed_repo(state, repo)?;
    let work_root = receive_pack_staging_repo_path(state, owner, repo_name)?;
    let public_work_repo = work_root.join("public-merge");
    let raw_work_repo = work_root.join("raw-apply");
    if work_root.exists() {
        fs::remove_dir_all(&work_root).map_err(ApiError::internal)?;
    }
    let result = async {
        ensure_private_dir(&work_root)?;
        run_git(
            None,
            &[
                "clone",
                "--no-hardlinks",
                raw_seed_repo.to_string_lossy().as_ref(),
                raw_work_repo.to_string_lossy().as_ref(),
            ],
            "cloning request raw merge worktree",
        )?;
        ensure_worktree_head(&raw_work_repo, current_main_oid)?;
        run_git(
            None,
            &[
                "clone",
                "--no-hardlinks",
                public_seed_repo.to_string_lossy().as_ref(),
                public_work_repo.to_string_lossy().as_ref(),
            ],
            "cloning request public merge worktree",
        )?;
        let public_base_oid = git_text(
            &public_work_repo,
            &["rev-parse", "HEAD"],
            "reading public merge main",
        )?;
        fetch_and_merge_request_branch(state, &public_work_repo, request, request_snapshot)?;
        apply_public_merge_to_raw_worktree(
            repo,
            &public_work_repo,
            &raw_work_repo,
            &public_base_oid,
            &request.id,
        )?;
        receive_pack_update_from_staging_repo(
            state,
            owner,
            repo_name,
            &raw_work_repo,
            maintainer_id,
            repo.repo_config.clone(),
        )
        .await
    }
    .await;
    let _ = fs::remove_dir_all(&work_root);
    result
}

fn ensure_merge_work_parent(work_repo: &FsPath) -> Result<(), ApiError> {
    if let Some(parent) = work_repo.parent() {
        ensure_private_dir(parent)?;
    }
    Ok(())
}

fn private_merge_seed_repo(
    state: &AppState,
    repo: &StoredRepository,
) -> Result<GitRepoHandle, ApiError> {
    if let Some(head) = repo.git_head.as_ref() {
        return cached_raw_git_repo(state, &head.manifest);
    }
    let projection = project_graph(
        &repo.policy,
        &repo.graph,
        &repo.visibility_events,
        ProjectionViewKey::Private,
    );
    projection_bare_repo_for_state(state, &projection).map(GitRepoHandle::from_path)
}

fn public_merge_seed_repo(
    state: &AppState,
    repo: &StoredRepository,
) -> Result<std::path::PathBuf, ApiError> {
    let projection = project_graph(
        &repo.policy,
        &repo.graph,
        &repo.visibility_events,
        ProjectionViewKey::Public,
    );
    if projection.commits.is_empty() {
        return Err(ApiError::conflict(
            "repo has no public main branch to merge",
        ));
    }
    projection_bare_repo_for_state(state, &projection)
}

fn ensure_worktree_head(work_repo: &FsPath, expected_main_oid: &str) -> Result<(), ApiError> {
    let actual_main = git_text(work_repo, &["rev-parse", "HEAD"], "reading merge main")?;
    if actual_main == expected_main_oid {
        Ok(())
    } else {
        Err(ApiError::conflict("main changed since merge was confirmed"))
    }
}

fn fetch_and_merge_request_branch(
    state: &AppState,
    work_repo: &FsPath,
    request: &Request,
    request_snapshot: &crate::domain::store::SourceBlob,
) -> Result<(), ApiError> {
    let request_bundle = work_repo.join(".git").join("scope-request.bundle.tmp");
    let request_bytes = source_blob_bytes(state.object_store.as_ref(), request_snapshot)?;
    fs::write(&request_bundle, request_bytes).map_err(ApiError::internal)?;
    let request_ref = canonical_request_ref(&request.name);
    let refspec = format!("{request_ref}:refs/remotes/scope/request");
    run_git(
        Some(work_repo),
        &["fetch", request_bundle.to_string_lossy().as_ref(), &refspec],
        "fetching request branch for merge",
    )?;
    let _ = fs::remove_file(&request_bundle);
    let actual_request_head = git_text(
        work_repo,
        &["rev-parse", "refs/remotes/scope/request"],
        "reading request branch head",
    )?;
    if actual_request_head != request.head_oid {
        return Err(ApiError::conflict(
            "request branch changed since merge was confirmed",
        ));
    }
    let output = run_git_output(
        Some(work_repo),
        &[
            "-c",
            "user.name=Scope",
            "-c",
            "user.email=scope@example.invalid",
            "merge",
            "--no-ff",
            "--no-edit",
            "refs/remotes/scope/request",
        ],
        "clean-merging request branch",
    )?;
    if !output.status.success() {
        return Err(ApiError::conflict(format!(
            "request branch does not cleanly merge into current main: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        )));
    }
    Ok(())
}

enum PublicMergeChange {
    Upsert(String),
    Delete(String),
}

fn apply_public_merge_to_raw_worktree(
    repo: &StoredRepository,
    public_work_repo: &FsPath,
    raw_work_repo: &FsPath,
    public_base_oid: &str,
    request_id: &str,
) -> Result<(), ApiError> {
    let changes = public_merge_changes(public_work_repo, public_base_oid)?;
    if changes.is_empty() {
        return Err(ApiError::bad_request(
            "request merge did not change the public tree",
        ));
    }
    let mut force_add_paths = Vec::new();
    for change in changes {
        match change {
            PublicMergeChange::Upsert(path) => {
                ensure_public_request_path(repo, &path)?;
                let source = public_work_repo.join(&path);
                let target = raw_work_repo.join(&path);
                if let Some(parent) = target.parent() {
                    fs::create_dir_all(parent).map_err(ApiError::internal)?;
                }
                fs::copy(&source, &target).map_err(ApiError::internal)?;
                apply_public_file_mode(public_work_repo, &target, &path)?;
                force_add_paths.push(path);
            }
            PublicMergeChange::Delete(path) => {
                ensure_public_request_path(repo, &path)?;
                let target = raw_work_repo.join(path);
                if target.exists() {
                    fs::remove_file(target).map_err(ApiError::internal)?;
                }
            }
        }
    }
    if !force_add_paths.is_empty() {
        let mut args = vec!["add", "-f", "--"];
        args.extend(force_add_paths.iter().map(String::as_str));
        run_git(
            Some(raw_work_repo),
            &args,
            "force-staging translated public request files",
        )?;
    }
    run_git(
        Some(raw_work_repo),
        &["add", "-A"],
        "staging translated public request merge",
    )?;
    let diff = run_git_output(
        Some(raw_work_repo),
        &["diff", "--cached", "--quiet"],
        "checking translated public request merge",
    )?;
    if diff.status.success() {
        return Err(ApiError::bad_request(
            "request merge did not change the raw tree",
        ));
    }
    if diff.status.code() != Some(1) {
        return Err(ApiError::service_unavailable(format!(
            "checking translated public request merge: {}",
            String::from_utf8_lossy(&diff.stderr).trim()
        )));
    }
    let message = format!("Merge request {request_id}");
    run_git(
        Some(raw_work_repo),
        &[
            "-c",
            "user.name=Scope",
            "-c",
            "user.email=scope@example.invalid",
            "commit",
            "-m",
            &message,
        ],
        "committing translated public request merge",
    )
}

fn ensure_public_request_path(repo: &StoredRepository, path: &str) -> Result<(), ApiError> {
    let scope_path = ScopePath::parse(format!("/{path}")).map_err(ApiError::bad_request)?;
    let public_projection = project_graph(
        &repo.policy,
        &repo.graph,
        &repo.visibility_events,
        ProjectionViewKey::Public,
    );
    if public_projection
        .visible_paths()
        .iter()
        .any(|path| path == scope_path.as_str())
    {
        return Ok(());
    }
    if repo.graph_has_file(&scope_path) {
        return Err(ApiError::conflict(
            "public request cannot change a private path",
        ));
    }
    if repo.policy.effective_visibility(&scope_path) == Visibility::Public {
        Ok(())
    } else {
        Err(ApiError::conflict(
            "public request cannot change a private path",
        ))
    }
}

fn public_merge_changes(
    public_work_repo: &FsPath,
    public_base_oid: &str,
) -> Result<Vec<PublicMergeChange>, ApiError> {
    let output = run_git_output(
        Some(public_work_repo),
        &[
            "diff",
            "--name-status",
            "-z",
            "--no-renames",
            public_base_oid,
            "HEAD",
        ],
        "reading public request merge diff",
    )?;
    if !output.status.success() {
        return Err(ApiError::service_unavailable(format!(
            "reading public request merge diff: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        )));
    }
    let mut parts = output.stdout.split(|byte| *byte == 0);
    let mut changes = Vec::new();
    while let Some(status) = parts.next() {
        if status.is_empty() {
            continue;
        }
        let Some(path) = parts.next() else {
            return Err(ApiError::internal_message(
                "public request merge diff is missing a path",
            ));
        };
        let path = String::from_utf8(path.to_vec()).map_err(ApiError::bad_request)?;
        match status[0] {
            b'A' | b'M' | b'T' => changes.push(PublicMergeChange::Upsert(path)),
            b'D' => changes.push(PublicMergeChange::Delete(path)),
            _ => {
                return Err(ApiError::internal_message(format!(
                    "unsupported public request merge diff status {}",
                    String::from_utf8_lossy(status)
                )));
            }
        }
    }
    Ok(changes)
}

fn apply_public_file_mode(
    public_work_repo: &FsPath,
    raw_path: &FsPath,
    path: &str,
) -> Result<(), ApiError> {
    let entry = git_text(
        public_work_repo,
        &["ls-tree", "HEAD", "--", path],
        "reading public request file mode",
    )?;
    if entry.is_empty() {
        return Err(ApiError::internal_message(
            "public request merge file is missing from merged tree",
        ));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = if entry.starts_with("100755 ") {
            0o755
        } else {
            0o644
        };
        fs::set_permissions(raw_path, fs::Permissions::from_mode(mode))
            .map_err(ApiError::internal)?;
    }
    Ok(())
}

fn git_text(repo: &FsPath, args: &[&str], action: &str) -> Result<String, ApiError> {
    let output = run_git_output(Some(repo), args, action)?;
    if !output.status.success() {
        return Err(ApiError::service_unavailable(format!(
            "{action}: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        )));
    }
    Ok(String::from_utf8(output.stdout)
        .map_err(ApiError::bad_request)?
        .trim()
        .to_string())
}
