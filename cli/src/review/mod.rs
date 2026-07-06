mod git_paths;
mod policy;
mod state;
mod tree;
mod tui;

use crate::{
    git_repo::{GitChangedPath, GitRepo},
    repo_config::{
        ensure_scope_repo_config_exists, load_scope_repo_config_at_commit,
        load_worktree_scope_repo_config, repo_config_path, write_worktree_scope_repo_config,
    },
};
use anyhow::bail;
use std::io::{self, IsTerminal};

use self::{
    git_paths::{committed_review_tree, worktree_review_tree},
    state::{ReviewMode, ReviewState},
    tui::{TuiOutcome, run_review_tui},
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PushReviewOutcome {
    Continue,
    ConfigChanged,
}

pub fn run_standalone_review(repo: &GitRepo) -> anyhow::Result<()> {
    ensure_review_terminal_available("scope review")?;
    ensure_scope_repo_config_exists(&repo.root)?;
    let config = load_worktree_scope_repo_config(&repo.root)?;
    let tree = worktree_review_tree(repo)?;
    let state = ReviewState::new(tree, config, ReviewMode::Standalone);

    match run_review_tui(state, |config| {
        write_worktree_scope_repo_config(&repo.root, config)
    })? {
        TuiOutcome::Exit | TuiOutcome::ConfigChanged => Ok(()),
        TuiOutcome::Cancel => bail!("scope review cancelled"),
        TuiOutcome::ContinuePush => Ok(()),
    }
}

pub fn run_push_review(
    repo: &GitRepo,
    reviewed_head_oid: &str,
    changed_paths: &[GitChangedPath],
) -> anyhow::Result<PushReviewOutcome> {
    ensure_review_terminal_available("scope push review")?;
    let config = load_scope_repo_config_at_commit(&repo.root, reviewed_head_oid)?;
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
        TuiOutcome::ContinuePush => Ok(PushReviewOutcome::Continue),
        TuiOutcome::ConfigChanged => Ok(PushReviewOutcome::ConfigChanged),
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

pub fn config_changed_message() -> String {
    format!(
        "{} changed. Commit it, then rerun scope push.",
        repo_config_path()
    )
}

fn deleted_path_summaries(changed_paths: &[GitChangedPath]) -> Vec<String> {
    changed_paths
        .iter()
        .filter(|path| path.status.starts_with('D'))
        .map(|path| format!("Deleted path: {} {}", path.status, path.path))
        .collect()
}
