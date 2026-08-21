use crate::{
    config::DEFAULT_GIT_BRANCH,
    error::ApiError,
    git::{
        import::{run_git, run_git_output, validate_pushed_tree},
        projection_repo::projection_bare_repo_for_state,
    },
    state::AppState,
};
use scope_domain::{
    policy::{ScopePath, Visibility},
    projection::{ProjectionViewKey, project_graph},
    repo_control::is_public_request_protected_path,
    store::{NativePublicCommit, StoredRepository},
};
use std::{collections::BTreeSet, path::Path as FsPath};

const PUBLIC_REQUEST_BASE_REF: &str = "refs/scope/internal/public-request-base";

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ValidatedPublicRequestRange {
    pub(crate) public_base_oid: String,
    pub(crate) public_parent_oids: Vec<String>,
    pub(crate) commits: Vec<NativePublicCommit>,
}

pub(super) fn ensure_public_request_ref_is_public_safe(
    repo: &StoredRepository,
    state: &AppState,
    staging_repo: &FsPath,
    new_head_oid: &str,
) -> Result<(), ApiError> {
    let (_, public_visible_paths) = fetch_current_public_projection(repo, state, staging_repo)?;
    public_request_branch_base_oid(staging_repo, new_head_oid)?;
    let commit_oids = commits_after(staging_repo, PUBLIC_REQUEST_BASE_REF, new_head_oid)?;
    validated_public_parent_oids(staging_repo, &commit_oids)?;
    for commit_oid in commit_oids {
        validate_pushed_tree(staging_repo, &commit_oid)?;
        ensure_public_request_commit_paths(repo, &public_visible_paths, staging_repo, &commit_oid)?;
    }
    Ok(())
}

pub(crate) fn validate_public_request_merge_range(
    repo: &StoredRepository,
    state: &AppState,
    staging_repo: &FsPath,
    request_head_oid: &str,
) -> Result<ValidatedPublicRequestRange, ApiError> {
    let (public_base_oid, public_visible_paths) =
        fetch_current_public_projection(repo, state, staging_repo)?;
    ensure_public_head_is_request_ancestor(staging_repo, request_head_oid)?;
    let commit_oids = commits_after(staging_repo, PUBLIC_REQUEST_BASE_REF, request_head_oid)?;
    if commit_oids.is_empty() {
        return Err(ApiError::conflict(
            "public request contains no commits after current public main",
        ));
    }
    let public_parent_oids = validated_public_parent_oids(staging_repo, &commit_oids)?;
    if !public_parent_oids.contains(&public_base_oid) {
        return Err(ApiError::conflict(
            "public main advanced; merge current public main into the request branch and push again",
        ));
    }

    let mut commits = Vec::with_capacity(commit_oids.len());
    for commit_oid in commit_oids {
        validate_pushed_tree(staging_repo, &commit_oid)?;
        let changed_paths = ensure_public_request_commit_paths(
            repo,
            &public_visible_paths,
            staging_repo,
            &commit_oid,
        )?;
        commits.push(public_request_commit_fact(
            staging_repo,
            &commit_oid,
            changed_paths,
        )?);
    }

    Ok(ValidatedPublicRequestRange {
        public_base_oid,
        public_parent_oids,
        commits,
    })
}

fn validated_public_parent_oids(
    staging_repo: &FsPath,
    commit_oids: &[String],
) -> Result<Vec<String>, ApiError> {
    let range_oids = commit_oids
        .iter()
        .map(String::as_str)
        .collect::<BTreeSet<_>>();
    let mut seen = BTreeSet::new();
    let mut public_parent_oids = BTreeSet::new();
    for commit_oid in commit_oids {
        let parent_oids = git_text(
            staging_repo,
            &["show", "-s", "--format=%P", commit_oid],
            "reading public request commit parents",
        )?
        .split_ascii_whitespace()
        .map(ToString::to_string)
        .collect::<Vec<_>>();
        if parent_oids.is_empty() {
            return Err(ApiError::conflict(
                "public request history must be based on public main",
            ));
        }
        for parent_oid in parent_oids {
            if range_oids.contains(parent_oid.as_str()) {
                if !seen.contains(parent_oid.as_str()) {
                    return Err(ApiError::conflict(
                        "public request commits are not ordered ancestor-first",
                    ));
                }
            } else if git_revision_is_ancestor(staging_repo, &parent_oid, PUBLIC_REQUEST_BASE_REF)?
            {
                public_parent_oids.insert(parent_oid);
            } else {
                return Err(ApiError::conflict(
                    "public request contains a parent outside public history; rewrite the branch onto public main and push again",
                ));
            }
        }
        seen.insert(commit_oid.as_str());
    }
    if public_parent_oids.is_empty() {
        return Err(ApiError::conflict(
            "public request history must be based on public main",
        ));
    }
    Ok(public_parent_oids.into_iter().collect())
}

fn git_revision_is_ancestor(
    staging_repo: &FsPath,
    ancestor: &str,
    descendant: &str,
) -> Result<bool, ApiError> {
    let output = run_git_output(
        Some(staging_repo),
        &["merge-base", "--is-ancestor", ancestor, descendant],
        "checking public request parent ancestry",
    )?;
    if output.status.success() {
        return Ok(true);
    }
    if output.status.code() == Some(1) {
        return Ok(false);
    }
    Err(ApiError::infrastructure_unavailable(format!(
        "checking public request parent ancestry: {}",
        String::from_utf8_lossy(&output.stderr).trim()
    )))
}

fn fetch_current_public_projection(
    repo: &StoredRepository,
    state: &AppState,
    staging_repo: &FsPath,
) -> Result<(String, BTreeSet<String>), ApiError> {
    let public_projection = project_graph(
        &repo.graph,
        &repo.visibility_change_sets,
        ProjectionViewKey::Public,
    );
    if public_projection.commits.is_empty() {
        return Err(ApiError::conflict(
            "repo has no public main branch for public request",
        ));
    }
    let public_visible_paths = public_projection
        .visible_paths()
        .into_iter()
        .collect::<BTreeSet<_>>();
    let public_repo = projection_bare_repo_for_state(
        state,
        &repo.record.id,
        &public_projection,
        repo.git_head.as_ref(),
        &repo.git_pack_spans,
    )?;
    let refspec = format!("+refs/heads/{DEFAULT_GIT_BRANCH}:{PUBLIC_REQUEST_BASE_REF}");
    run_git(
        Some(staging_repo),
        &[
            "fetch",
            public_repo.to_string_lossy().as_ref(),
            refspec.as_str(),
        ],
        "fetching public request base",
    )?;
    let public_base_oid = git_commit_oid(staging_repo, PUBLIC_REQUEST_BASE_REF)?;
    Ok((public_base_oid, public_visible_paths))
}

fn public_request_branch_base_oid(
    staging_repo: &FsPath,
    new_head_oid: &str,
) -> Result<String, ApiError> {
    let output = run_git_output(
        Some(staging_repo),
        &["merge-base", PUBLIC_REQUEST_BASE_REF, new_head_oid],
        "checking public request branch base",
    )?;
    if !output.status.success() {
        return Err(ApiError::conflict(
            "public request branch must be based on public main",
        ));
    }
    Ok(String::from_utf8(output.stdout)
        .map_err(ApiError::bad_request)?
        .trim()
        .to_string())
}

fn commits_after(
    staging_repo: &FsPath,
    base: &str,
    new_head_oid: &str,
) -> Result<Vec<String>, ApiError> {
    let exclude_base = format!("^{base}");
    let output = run_git_output(
        Some(staging_repo),
        &[
            "rev-list",
            "--reverse",
            "--topo-order",
            new_head_oid,
            exclude_base.as_str(),
        ],
        "reading public request branch commits",
    )?;
    if !output.status.success() {
        return Err(ApiError::infrastructure_unavailable(format!(
            "reading public request branch commits: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        )));
    }
    Ok(String::from_utf8(output.stdout)
        .map_err(ApiError::bad_request)?
        .lines()
        .filter(|line| !line.trim().is_empty())
        .map(ToString::to_string)
        .collect())
}

fn ensure_public_head_is_request_ancestor(
    staging_repo: &FsPath,
    request_head_oid: &str,
) -> Result<(), ApiError> {
    let output = run_git_output(
        Some(staging_repo),
        &[
            "merge-base",
            "--is-ancestor",
            PUBLIC_REQUEST_BASE_REF,
            request_head_oid,
        ],
        "checking current public main ancestry",
    )?;
    if output.status.success() {
        return Ok(());
    }
    Err(ApiError::conflict(
        "public main advanced; merge current public main into the request branch and push again",
    ))
}

fn public_request_commit_fact(
    staging_repo: &FsPath,
    commit_oid: &str,
    changed_paths: Vec<ScopePath>,
) -> Result<NativePublicCommit, ApiError> {
    let tree_oid = git_text(
        staging_repo,
        &["show", "-s", "--format=%T", commit_oid],
        "reading public request commit tree",
    )?;
    let parents = git_text(
        staging_repo,
        &["show", "-s", "--format=%P", commit_oid],
        "reading public request commit parents",
    )?;
    Ok(NativePublicCommit {
        oid: commit_oid.to_string(),
        parent_oids: parents
            .split_ascii_whitespace()
            .map(ToString::to_string)
            .collect(),
        tree_oid,
        changed_paths,
    })
}

fn git_commit_oid(staging_repo: &FsPath, revision: &str) -> Result<String, ApiError> {
    git_text(
        staging_repo,
        &["rev-parse", "--verify", &format!("{revision}^{{commit}}")],
        "reading current public main",
    )
}

fn git_text(staging_repo: &FsPath, args: &[&str], context: &str) -> Result<String, ApiError> {
    let output = run_git_output(Some(staging_repo), args, context)?;
    if !output.status.success() {
        return Err(ApiError::infrastructure_unavailable(format!(
            "{context}: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        )));
    }
    String::from_utf8(output.stdout)
        .map_err(ApiError::bad_request)
        .map(|value| value.trim().to_string())
}

fn ensure_public_request_commit_paths(
    repo: &StoredRepository,
    public_visible_paths: &BTreeSet<String>,
    staging_repo: &FsPath,
    commit_oid: &str,
) -> Result<Vec<ScopePath>, ApiError> {
    let mut changed_paths = BTreeSet::new();
    for path in public_request_changed_paths(staging_repo, commit_oid)? {
        changed_paths.insert(ensure_public_request_path(
            repo,
            public_visible_paths,
            &path,
        )?);
    }
    Ok(changed_paths.into_iter().collect())
}

fn public_request_changed_paths(
    staging_repo: &FsPath,
    commit_oid: &str,
) -> Result<Vec<String>, ApiError> {
    let public_base_oid = git_commit_oid(staging_repo, PUBLIC_REQUEST_BASE_REF)?;
    let parents = git_text(
        staging_repo,
        &["show", "-s", "--format=%P", commit_oid],
        "reading public request commit parents",
    )?
    .split_ascii_whitespace()
    .map(ToString::to_string)
    .collect::<Vec<_>>();
    let diff_base = parents
        .iter()
        .find(|parent| parent.as_str() == public_base_oid)
        .or_else(|| parents.first())
        .ok_or_else(|| ApiError::conflict("public request commit must have a parent"))?;
    let output = run_git_output(
        Some(staging_repo),
        &[
            "diff",
            "-r",
            "--name-only",
            "-z",
            "--no-renames",
            diff_base,
            commit_oid,
        ],
        "reading public request commit paths",
    )?;
    if !output.status.success() {
        return Err(ApiError::infrastructure_unavailable(format!(
            "reading public request commit paths: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        )));
    }
    let mut changed_paths = Vec::new();
    for path in output.stdout.split(|byte| *byte == 0) {
        if path.is_empty() {
            continue;
        }
        let path = String::from_utf8(path.to_vec()).map_err(ApiError::bad_request)?;
        changed_paths.push(path);
    }
    Ok(changed_paths)
}

fn ensure_public_request_path(
    repo: &StoredRepository,
    public_visible_paths: &BTreeSet<String>,
    path: &str,
) -> Result<ScopePath, ApiError> {
    let scope_path = ScopePath::parse(format!("/{path}")).map_err(ApiError::bad_request)?;
    if is_public_request_protected_path(&scope_path) {
        return Err(ApiError::protected_paths(vec![path.to_string()]));
    }
    if public_visible_paths
        .iter()
        .any(|path| path == scope_path.as_str())
    {
        return Ok(scope_path);
    }
    if repo.graph_has_file(&scope_path) {
        return Err(ApiError::conflict(
            "public request cannot change a private path",
        ));
    }
    if repo_path_has_private_history(repo, &scope_path) {
        return Err(ApiError::conflict(
            "public request cannot change a private path",
        ));
    }
    if repo.repo_config.visibility_for_path(&scope_path) == Visibility::Public {
        Ok(scope_path)
    } else {
        Err(ApiError::conflict(
            "public request cannot change a private path",
        ))
    }
}

fn repo_path_has_private_history(repo: &StoredRepository, scope_path: &ScopePath) -> bool {
    repo.graph
        .commits
        .iter()
        .flat_map(|commit| &commit.changes)
        .any(|change| {
            change.path.as_str() == scope_path.as_str() && change.visibility == Visibility::Private
        })
        || repo
            .visibility_change_sets
            .iter()
            .flat_map(|set| &set.changes)
            .any(|change| {
                change.path.as_str() == scope_path.as_str()
                    && (change.old_visibility == Visibility::Private
                        || change.new_visibility == Visibility::Private)
            })
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
    fn public_request_range_is_oldest_first_with_exact_git_facts() {
        let repo = initialized_repo("exact-range");
        fs::write(repo.join("public.txt"), "base\n").unwrap();
        commit_all(&repo, "public base");
        let public_base = oid(&repo, "HEAD");
        run_git(
            Some(&repo),
            &["update-ref", PUBLIC_REQUEST_BASE_REF, &public_base],
            "recording public request base",
        )
        .unwrap();

        fs::write(repo.join("public.txt"), "first\n").unwrap();
        commit_all(&repo, "first request commit");
        let first = oid(&repo, "HEAD");
        fs::write(repo.join("second.txt"), "second\n").unwrap();
        commit_all(&repo, "second request commit");
        let second = oid(&repo, "HEAD");

        let commits = commits_after(&repo, PUBLIC_REQUEST_BASE_REF, &second).unwrap();
        assert_eq!(commits, [first.clone(), second.clone()]);

        let first_path = ScopePath::parse("/public.txt").unwrap();
        let first_fact =
            public_request_commit_fact(&repo, &first, vec![first_path.clone()]).unwrap();
        assert_eq!(first_fact.oid, first);
        assert_eq!(first_fact.parent_oids, [public_base]);
        assert_eq!(first_fact.changed_paths, [first_path]);
        assert_eq!(
            first_fact.tree_oid,
            oid(&repo, &format!("{}^{{tree}}", first_fact.oid))
        );

        let second_path = ScopePath::parse("/second.txt").unwrap();
        let second_fact =
            public_request_commit_fact(&repo, &second, vec![second_path.clone()]).unwrap();
        assert_eq!(second_fact.oid, second);
        assert_eq!(second_fact.parent_oids, [first_fact.oid]);
        assert_eq!(second_fact.changed_paths, [second_path]);
        assert_eq!(
            second_fact.tree_oid,
            oid(&repo, &format!("{}^{{tree}}", second_fact.oid))
        );

        let _ = fs::remove_dir_all(repo);
    }

    #[test]
    fn public_request_range_rejects_parent_outside_public_history_or_range() {
        let repo = initialized_repo("external-parent");
        fs::write(repo.join("public.txt"), "base\n").unwrap();
        commit_all(&repo, "public base");
        let public_base = oid(&repo, "HEAD");
        run_git(
            Some(&repo),
            &["update-ref", PUBLIC_REQUEST_BASE_REF, &public_base],
            "recording public request base",
        )
        .unwrap();

        run_git(
            Some(&repo),
            &["switch", "--create", "external"],
            "creating external branch",
        )
        .unwrap();
        fs::write(repo.join("external.txt"), "external\n").unwrap();
        commit_all(&repo, "external commit");

        run_git(
            Some(&repo),
            &["switch", "--create", "request", &public_base],
            "creating request branch",
        )
        .unwrap();
        fs::write(repo.join("request.txt"), "request\n").unwrap();
        commit_all(&repo, "request commit");
        let request_commit = oid(&repo, "HEAD");
        run_git(
            Some(&repo),
            &[
                "merge",
                "--no-ff",
                "external",
                "-m",
                "merge external parent",
            ],
            "creating request merge with external parent",
        )
        .unwrap();
        let request_head = oid(&repo, "HEAD");

        let error =
            validated_public_parent_oids(&repo, &[request_commit, request_head]).unwrap_err();
        assert!(
            error
                .public_message()
                .contains("parent outside public history")
        );

        let _ = fs::remove_dir_all(repo);
    }

    #[test]
    fn merge_validation_rejects_request_without_current_public_head() {
        let repo = initialized_repo("stale-public-head");
        fs::write(repo.join("public.txt"), "base\n").unwrap();
        commit_all(&repo, "public base");
        let original_base = oid(&repo, "HEAD");

        run_git(
            Some(&repo),
            &["switch", "--create", "request", &original_base],
            "creating request branch",
        )
        .unwrap();
        fs::write(repo.join("request.txt"), "request\n").unwrap();
        commit_all(&repo, "request change");
        let request_head = oid(&repo, "HEAD");

        run_git(Some(&repo), &["switch", "main"], "returning to public main").unwrap();
        fs::write(repo.join("main.txt"), "advanced\n").unwrap();
        commit_all(&repo, "advance public main");
        let current_public_head = oid(&repo, "HEAD");
        run_git(
            Some(&repo),
            &["update-ref", PUBLIC_REQUEST_BASE_REF, &current_public_head],
            "recording advanced public request base",
        )
        .unwrap();

        assert!(ensure_public_head_is_request_ancestor(&repo, &request_head).is_err());

        let _ = fs::remove_dir_all(repo);
    }

    #[test]
    fn merge_path_validation_ignores_rules_inherited_from_current_public_main() {
        let repo = initialized_repo("inherited-public-rules");
        fs::write(repo.join("public.txt"), "base\n").unwrap();
        commit_all(&repo, "public base");
        let original_base = oid(&repo, "HEAD");

        run_git(
            Some(&repo),
            &["switch", "--create", "request", &original_base],
            "creating request branch",
        )
        .unwrap();
        fs::write(repo.join("request.txt"), "request\n").unwrap();
        commit_all(&repo, "request change");

        run_git(Some(&repo), &["switch", "main"], "returning to public main").unwrap();
        fs::write(repo.join(".scope/RULES.md"), "maintainer rules\n").unwrap();
        commit_all(&repo, "update maintainer rules");
        let current_public_head = oid(&repo, "HEAD");
        run_git(
            Some(&repo),
            &["update-ref", PUBLIC_REQUEST_BASE_REF, &current_public_head],
            "recording advanced public request base",
        )
        .unwrap();

        run_git(
            Some(&repo),
            &["switch", "request"],
            "returning to request branch",
        )
        .unwrap();
        run_git(
            Some(&repo),
            &[
                "merge",
                "--no-ff",
                "main",
                "-m",
                "merge current public main",
            ],
            "merging current public main",
        )
        .unwrap();
        let merge_oid = oid(&repo, "HEAD");

        let paths = public_request_changed_paths(&repo, &merge_oid).unwrap();

        assert_eq!(paths, ["request.txt"]);

        fs::write(repo.join(".scope/RULES.md"), "request override\n").unwrap();
        run_git(
            Some(&repo),
            &["add", ".scope/RULES.md"],
            "staging request rules override",
        )
        .unwrap();
        run_git(
            Some(&repo),
            &["commit", "--amend", "--no-edit"],
            "amending merge with request rules override",
        )
        .unwrap();
        let amended_merge_oid = oid(&repo, "HEAD");

        assert_eq!(
            public_request_changed_paths(&repo, &amended_merge_oid).unwrap(),
            [".scope/RULES.md", "request.txt"]
        );

        let _ = fs::remove_dir_all(repo);
    }

    fn initialized_repo(label: &str) -> PathBuf {
        let repo = temp_repo_path(label);
        run_git(
            None,
            &[
                "init",
                "--initial-branch=main",
                repo.to_string_lossy().as_ref(),
            ],
            "initializing public request safety test repository",
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
        fs::create_dir_all(repo.join(".scope")).unwrap();
        fs::write(repo.join(".scope/RULES.md"), []).unwrap();
        repo
    }

    fn commit_all(repo: &Path, message: &str) {
        run_git(Some(repo), &["add", "."], "staging safety test files").unwrap();
        run_git(
            Some(repo),
            &["commit", "-m", message],
            "committing safety test files",
        )
        .unwrap();
    }

    fn oid(repo: &Path, revision: &str) -> String {
        git_text(repo, &["rev-parse", revision], "reading safety test oid").unwrap()
    }

    fn temp_repo_path(label: &str) -> PathBuf {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "scope-vcs-public-request-safety-{label}-{}-{nonce}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&path);
        path
    }
}
