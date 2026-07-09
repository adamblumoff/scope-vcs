use crate::{
    config::DEFAULT_GIT_BRANCH,
    domain::{
        policy::{ScopePath, Visibility},
        projection::{ProjectionViewKey, project_graph},
        store::StoredRepository,
    },
    error::ApiError,
    git::{
        import::{run_git, run_git_output},
        upload::projection_bare_repo_for_state,
    },
    state::AppState,
};
use std::{collections::BTreeSet, path::Path as FsPath};

const PUBLIC_REQUEST_BASE_REF: &str = "refs/scope/internal/public-request-base";

pub(super) fn ensure_public_request_ref_is_public_safe(
    repo: &StoredRepository,
    state: &AppState,
    staging_repo: &FsPath,
    new_head_oid: &str,
) -> Result<(), ApiError> {
    let public_projection = project_graph(
        &repo.policy,
        &repo.graph,
        &repo.visibility_events,
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
    let public_repo = projection_bare_repo_for_state(state, &public_projection)?;
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
    let public_base_oid = public_request_branch_base_oid(staging_repo, new_head_oid)?;
    for commit_oid in public_request_branch_commits(staging_repo, new_head_oid)? {
        ensure_public_request_commit_descends_from_base(
            staging_repo,
            &public_base_oid,
            &commit_oid,
        )?;
        ensure_public_request_commit_paths(repo, &public_visible_paths, staging_repo, &commit_oid)?;
    }
    Ok(())
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

fn public_request_branch_commits(
    staging_repo: &FsPath,
    new_head_oid: &str,
) -> Result<Vec<String>, ApiError> {
    let exclude_public_main = format!("^{PUBLIC_REQUEST_BASE_REF}");
    let output = run_git_output(
        Some(staging_repo),
        &[
            "rev-list",
            "--topo-order",
            new_head_oid,
            exclude_public_main.as_str(),
        ],
        "reading public request branch commits",
    )?;
    if !output.status.success() {
        return Err(ApiError::service_unavailable(format!(
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

fn ensure_public_request_commit_descends_from_base(
    staging_repo: &FsPath,
    public_base_oid: &str,
    commit_oid: &str,
) -> Result<(), ApiError> {
    let output = run_git_output(
        Some(staging_repo),
        &["merge-base", "--is-ancestor", public_base_oid, commit_oid],
        "checking public request commit ancestry",
    )?;
    if output.status.success() {
        return Ok(());
    }
    Err(ApiError::conflict(
        "public request branch cannot include private history",
    ))
}

fn ensure_public_request_commit_paths(
    repo: &StoredRepository,
    public_visible_paths: &BTreeSet<String>,
    staging_repo: &FsPath,
    commit_oid: &str,
) -> Result<(), ApiError> {
    let output = run_git_output(
        Some(staging_repo),
        &[
            "diff-tree",
            "--root",
            "-r",
            "-m",
            "--no-commit-id",
            "--name-only",
            "-z",
            "--no-renames",
            commit_oid,
        ],
        "reading public request commit paths",
    )?;
    if !output.status.success() {
        return Err(ApiError::service_unavailable(format!(
            "reading public request commit paths: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        )));
    }
    for path in output.stdout.split(|byte| *byte == 0) {
        if path.is_empty() {
            continue;
        }
        let path = String::from_utf8(path.to_vec()).map_err(ApiError::bad_request)?;
        ensure_public_request_path(repo, public_visible_paths, &path)?;
    }
    Ok(())
}

fn ensure_public_request_path(
    repo: &StoredRepository,
    public_visible_paths: &BTreeSet<String>,
    path: &str,
) -> Result<(), ApiError> {
    let scope_path = ScopePath::parse(format!("/{path}")).map_err(ApiError::bad_request)?;
    if public_visible_paths
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
    if repo_path_has_private_history(repo, &scope_path) {
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

fn repo_path_has_private_history(repo: &StoredRepository, scope_path: &ScopePath) -> bool {
    repo.graph
        .commits
        .iter()
        .flat_map(|commit| &commit.changes)
        .any(|change| {
            change.path.as_str() == scope_path.as_str() && change.visibility == Visibility::Private
        })
        || repo.visibility_events.iter().any(|event| {
            event.path.as_str() == scope_path.as_str()
                && (event.old_visibility == Visibility::Private
                    || event.new_visibility == Visibility::Private)
        })
}
