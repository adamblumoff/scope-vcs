use crate::git_repo::{
    GitChangedPath, GitRepo, committed_file_paths_at_commit, worktree_file_paths,
};

use super::tree::ReviewTree;

pub fn worktree_review_tree(repo: &GitRepo) -> anyhow::Result<ReviewTree> {
    let paths = worktree_file_paths(repo)?;
    Ok(ReviewTree::from_paths(&paths, &[]))
}

pub fn committed_review_tree(
    repo: &GitRepo,
    commit_oid: &str,
    changed_paths: &[GitChangedPath],
) -> anyhow::Result<ReviewTree> {
    let paths = committed_file_paths_at_commit(repo, commit_oid)?;
    Ok(ReviewTree::from_paths(&paths, changed_paths))
}
