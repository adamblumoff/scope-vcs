use crate::{
    config::DEFAULT_GIT_BRANCH,
    domain::{
        policy::{ScopePath, Visibility},
        repo_config::RepoConfig,
        requests::{Request, RequestAudience, canonical_request_ref},
        store::{SourceBlob, StoredRepository},
    },
    error::ApiError,
    git::{
        import::{
            ReceivePackUpdate, request_merge_update_from_staging_repo, run_git, run_git_output,
        },
        request_refs::attach_visible_request_refs,
        storage::{cached_raw_git_repo, receive_pack_staging_repo_path, remove_dir_if_exists},
        upload::git_index_command,
    },
    persistence::ensure_private_dir,
    state::AppState,
};
use std::{fs, process::Command};

pub(crate) struct PreparedRequestMerge {
    pub(crate) expected_manifest_key: String,
    pub(crate) expected_repo_change_version: u64,
    pub(crate) prepared_request_head_oid: String,
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
        let merged_main_oid = merge_main_oid(
            &staging_repo,
            &request.base_main_oid,
            &current.head_oid,
            &request.head_oid,
            &request.name,
            (request.audience == RequestAudience::Public).then_some(&repo.repo_config),
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
        Ok(PreparedRequestMerge {
            expected_manifest_key: current.manifest.object_key.clone(),
            expected_repo_change_version: repo.record.change_version,
            prepared_request_head_oid: request.head_oid.clone(),
            update,
        })
    }
    .await;
    let cleanup = remove_dir_if_exists(&staging_repo);
    match (prepared, cleanup) {
        (Ok(value), Ok(())) => Ok(value),
        (Err(error), _) => Err(error),
        (Ok(_), Err(error)) => Err(error),
    }
}

fn merge_main_oid(
    repo: &std::path::Path,
    request_base_oid: &str,
    current_main_oid: &str,
    request_head_oid: &str,
    request_name: &str,
    public_request_config: Option<&RepoConfig>,
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
    let merge_head_oid = match public_request_config {
        Some(config) => {
            current_policy_public_request_head(repo, request_base_oid, request_head_oid, config)?
        }
        None => request_head_oid.to_string(),
    };
    let merge_tree = run_git_output(
        Some(repo),
        &[
            "merge-tree",
            "--write-tree",
            &format!("--merge-base={request_base_oid}"),
            current_main_oid,
            &merge_head_oid,
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

fn current_policy_public_request_head(
    repo: &std::path::Path,
    request_base_oid: &str,
    request_head_oid: &str,
    config: &RepoConfig,
) -> Result<String, ApiError> {
    let changed = run_git_output(
        Some(repo),
        &[
            "diff",
            "--name-only",
            "-z",
            "--no-renames",
            request_base_oid,
            request_head_oid,
        ],
        "reading public request paths",
    )?;
    if !changed.status.success() {
        return Err(ApiError::service_unavailable(format!(
            "reading public request paths: {}",
            String::from_utf8_lossy(&changed.stderr).trim()
        )));
    }
    let mut private_paths = Vec::new();
    for raw_path in changed.stdout.split(|byte| *byte == 0) {
        if raw_path.is_empty() {
            continue;
        }
        let path = String::from_utf8(raw_path.to_vec()).map_err(ApiError::bad_request)?;
        let scope_path = ScopePath::parse(format!("/{path}")).map_err(ApiError::bad_request)?;
        if config.visibility_for_path(&scope_path) == Visibility::Private {
            private_paths.push(path);
        }
    }
    if private_paths.is_empty() {
        return Ok(request_head_oid.to_string());
    }

    let index_path = repo.join("scope-public-request-merge.index");
    if index_path.exists() {
        fs::remove_file(&index_path).map_err(ApiError::internal)?;
    }
    let result = (|| {
        git_index_command(
            Command::new("git")
                .arg("-C")
                .arg(repo)
                .arg("read-tree")
                .arg(request_head_oid),
            &index_path,
            None,
        )?;
        for path in private_paths {
            restore_index_path_from_tree(repo, &index_path, request_base_oid, &path)?;
        }
        let tree_oid = git_index_command(
            Command::new("git").arg("-C").arg(repo).arg("write-tree"),
            &index_path,
            None,
        )?;
        let tree_oid = String::from_utf8(tree_oid).map_err(ApiError::bad_request)?;
        let commit = run_git_output(
            Some(repo),
            &[
                "commit-tree",
                tree_oid.trim(),
                "-p",
                request_base_oid,
                "-m",
                "Apply public request under current visibility policy",
            ],
            "creating policy-masked public request commit",
        )?;
        if !commit.status.success() {
            return Err(ApiError::service_unavailable(format!(
                "creating policy-masked public request commit: {}",
                String::from_utf8_lossy(&commit.stderr).trim()
            )));
        }
        String::from_utf8(commit.stdout)
            .map_err(ApiError::internal)
            .map(|oid| oid.trim().to_string())
    })();
    let cleanup = if index_path.exists() {
        fs::remove_file(index_path).map_err(ApiError::internal)
    } else {
        Ok(())
    };
    match (result, cleanup) {
        (Ok(value), Ok(())) => Ok(value),
        (Err(error), _) => Err(error),
        (Ok(_), Err(error)) => Err(error),
    }
}

fn restore_index_path_from_tree(
    repo: &std::path::Path,
    index_path: &std::path::Path,
    tree_oid: &str,
    path: &str,
) -> Result<(), ApiError> {
    let entry = run_git_output(
        Some(repo),
        &["ls-tree", "-z", tree_oid, "--", path],
        "reading public request base path",
    )?;
    if !entry.status.success() {
        return Err(ApiError::service_unavailable(format!(
            "reading public request base path: {}",
            String::from_utf8_lossy(&entry.stderr).trim()
        )));
    }
    if entry.stdout.is_empty() {
        let deletion = format!("0 0000000000000000000000000000000000000000\t{path}\0");
        git_index_command(
            Command::new("git")
                .arg("-C")
                .arg(repo)
                .arg("update-index")
                .arg("-z")
                .arg("--index-info"),
            index_path,
            Some(deletion.as_bytes()),
        )?;
    } else {
        git_index_command(
            Command::new("git")
                .arg("-C")
                .arg(repo)
                .arg("update-index")
                .arg("-z")
                .arg("--index-info"),
            index_path,
            Some(&entry.stdout),
        )?;
    }
    Ok(())
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
            None,
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
        let _ = fs::remove_dir_all(repo);
    }

    #[test]
    fn public_request_cannot_change_path_made_private_after_its_base() {
        let repo = temp_repo_path("current-private");
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
        fs::write(repo.join("private-later.txt"), "visible at request base\n").unwrap();
        commit_all(&repo, "public base");
        let request_base = oid(&repo, "HEAD");

        fs::write(repo.join("main-only.txt"), "current main\n").unwrap();
        fs::write(repo.join("private-later.txt"), "private current\n").unwrap();
        commit_all(&repo, "make path private in current policy");
        let current_main = oid(&repo, "HEAD");

        run_git(
            Some(&repo),
            &["switch", "--create", "request", &request_base],
            "creating request branch",
        )
        .unwrap();
        fs::write(repo.join("public.txt"), "public request\n").unwrap();
        fs::write(repo.join("private-later.txt"), "request overwrite\n").unwrap();
        commit_all(&repo, "request changes");
        let request_head = oid(&repo, "HEAD");

        let mut current_config = RepoConfig::with_default_visibility(
            crate::domain::repo_config::ConfigVisibility::Public,
        );
        current_config.visibility.rules.push(
            crate::domain::repo_config::RepoConfigVisibilityRule {
                path: "/private-later.txt".to_string(),
                visibility: crate::domain::repo_config::ConfigVisibility::Private,
            },
        );
        let merged = merge_main_oid(
            &repo,
            &request_base,
            &current_main,
            &request_head,
            "public-request",
            Some(&current_config),
        )
        .unwrap();
        assert_eq!(
            git_text(&repo, &["show", &format!("{merged}:private-later.txt")]),
            "private current\n"
        );
        assert_eq!(
            git_text(&repo, &["show", &format!("{merged}:public.txt")]),
            "public request\n"
        );
        assert_eq!(
            git_text(&repo, &["show", &format!("{merged}:main-only.txt")]),
            "current main\n"
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
