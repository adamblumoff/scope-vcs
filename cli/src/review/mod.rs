mod git_paths;
mod policy;
mod state;
mod tree;
mod tui;

use crate::{
    git_repo::{GitChangedPath, GitRepo},
    repo_config::{
        ensure_scope_repo_config_exists, load_worktree_scope_repo_config,
        write_worktree_scope_repo_config,
    },
};
use anyhow::bail;
use scope_core::domain::repo_config::RepoConfig;
use std::io::{self, IsTerminal};

use self::{
    git_paths::{committed_review_tree, worktree_review_tree},
    state::{ReviewMode, ReviewState},
    tui::{TuiOutcome, run_review_tui},
};

pub fn run_standalone_review(repo: &GitRepo) -> anyhow::Result<()> {
    ensure_review_terminal_available("scope review")?;
    ensure_scope_repo_config_exists(&repo.root)?;
    let config = load_worktree_scope_repo_config(&repo.root)?;
    let tree = worktree_review_tree(repo)?;
    let state = ReviewState::new(tree, config, ReviewMode::Standalone);

    match run_review_tui(state, |config| {
        write_worktree_scope_repo_config(&repo.root, config)
    })? {
        TuiOutcome::Exit => Ok(()),
        TuiOutcome::Cancel => bail!("scope review cancelled"),
        TuiOutcome::ContinuePush => Ok(()),
    }
}

pub fn run_push_review(
    repo: &GitRepo,
    reviewed_head_oid: &str,
    changed_paths: &[GitChangedPath],
) -> anyhow::Result<RepoConfig> {
    ensure_review_terminal_available("scope push review")?;
    let config = load_worktree_scope_repo_config(&repo.root)?;
    let tree = committed_review_tree(repo, reviewed_head_oid, changed_paths)?;
    let state = ReviewState::new_with_deleted_paths(
        tree,
        config,
        ReviewMode::Push,
        deleted_path_summaries(changed_paths),
    );

    match run_review_tui(state, |config| {
        write_worktree_scope_repo_config(&repo.root, config)
    })? {
        TuiOutcome::ContinuePush => load_worktree_scope_repo_config(&repo.root),
        TuiOutcome::Exit | TuiOutcome::Cancel => bail!("scope push cancelled"),
    }
}

pub fn ensure_review_terminal_available(command_name: &str) -> anyhow::Result<()> {
    if io::stdin().is_terminal() && io::stdout().is_terminal() {
        return Ok(());
    }

    bail!(
        "{command_name} requires an interactive terminal; use --no-review with scope push to skip review"
    )
}

fn deleted_path_summaries(changed_paths: &[GitChangedPath]) -> Vec<String> {
    changed_paths
        .iter()
        .filter(|path| path.status.starts_with('D'))
        .map(|path| format!("Deleted path: {} {}", path.status, path.path))
        .collect()
}
