use super::{
    policy::{Policy, Principal, ScopePath, Visibility},
    projection::{ProjectedCommit, VisibilityEvent, project_graph},
    store::{SourceBlob, StagedFileChangeKind},
};
use std::collections::BTreeMap;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct CommitHistoryView {
    pub(crate) repo_id: String,
    pub(crate) principal_id: String,
    pub(crate) commits: Vec<CommitHistoryCommit>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct CommitHistoryCommit {
    pub(crate) projected_id: String,
    pub(crate) logical_commit_id: String,
    pub(crate) parent_projected_id: Option<String>,
    pub(crate) author: Option<String>,
    pub(crate) message: String,
    pub(crate) files: Vec<CommitHistoryFile>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct CommitHistoryFile {
    pub(crate) path: ScopePath,
    pub(crate) kind: StagedFileChangeKind,
    pub(crate) old_content: Option<SourceBlob>,
    pub(crate) new_content: Option<SourceBlob>,
    pub(crate) visibility: Visibility,
}

pub(crate) fn commit_history_view(
    policy: &Policy,
    graph: &super::projection::SourceGraph,
    visibility_events: &[VisibilityEvent],
    principal: &Principal,
) -> CommitHistoryView {
    let projection = project_graph(policy, graph, visibility_events, principal);
    let mut tree = BTreeMap::new();
    let commits = projection
        .commits
        .into_iter()
        .map(|commit| commit_history_commit(policy, &mut tree, commit))
        .collect();

    CommitHistoryView {
        repo_id: graph.repo_id.clone(),
        principal_id: principal.id.clone(),
        commits,
    }
}

fn commit_history_commit(
    policy: &Policy,
    tree: &mut BTreeMap<ScopePath, SourceBlob>,
    commit: ProjectedCommit,
) -> CommitHistoryCommit {
    let files = commit
        .changes
        .into_iter()
        .filter_map(|change| {
            let old_content = tree.get(&change.path).cloned();
            let new_content = change.new_content;
            let kind = file_change_kind(old_content.as_ref(), new_content.as_ref())?;

            match &new_content {
                Some(blob) => {
                    tree.insert(change.path.clone(), blob.clone());
                }
                None => {
                    tree.remove(&change.path);
                }
            }

            Some(CommitHistoryFile {
                visibility: policy.effective_visibility(&change.path),
                path: change.path,
                kind,
                old_content,
                new_content,
            })
        })
        .collect();

    CommitHistoryCommit {
        projected_id: commit.projected_id,
        logical_commit_id: commit.logical_commit_id,
        parent_projected_id: commit.parent_projected_id,
        author: commit.author,
        message: commit.message,
        files,
    }
}

fn file_change_kind(
    old_content: Option<&SourceBlob>,
    new_content: Option<&SourceBlob>,
) -> Option<StagedFileChangeKind> {
    match (old_content, new_content) {
        (None, Some(_)) => Some(StagedFileChangeKind::Added),
        (Some(_), Some(_)) => Some(StagedFileChangeKind::Modified),
        (Some(_), None) => Some(StagedFileChangeKind::Deleted),
        (None, None) => None,
    }
}
