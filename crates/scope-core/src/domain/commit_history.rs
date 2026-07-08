use super::{
    policy::{Policy, ScopePath, Visibility},
    projection::{ProjectedCommit, ProjectionViewKey, VisibilityEvent, project_graph},
    store::{FileChangeKind, SourceBlob},
};
use std::collections::BTreeMap;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CommitHistoryView {
    pub repo_id: String,
    pub view_key: String,
    pub commits: Vec<CommitHistoryCommit>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CommitHistoryCommit {
    pub projected_id: String,
    pub logical_commit_id: String,
    pub parent_projected_id: Option<String>,
    pub author: Option<String>,
    pub message: String,
    pub files: Vec<CommitHistoryFile>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CommitHistoryFile {
    pub path: ScopePath,
    pub kind: FileChangeKind,
    pub old_content: Option<SourceBlob>,
    pub new_content: Option<SourceBlob>,
    pub visibility: Visibility,
}

pub fn commit_history_view(
    policy: &Policy,
    graph: &super::projection::SourceGraph,
    visibility_events: &[VisibilityEvent],
    view_key: ProjectionViewKey,
) -> CommitHistoryView {
    let projection = project_graph(policy, graph, visibility_events, view_key);
    let mut tree = BTreeMap::new();
    let commits = projection
        .commits
        .into_iter()
        .map(|commit| commit_history_commit(&mut tree, commit))
        .collect();

    CommitHistoryView {
        repo_id: graph.repo_id.clone(),
        view_key: projection.view_key.as_str().to_string(),
        commits,
    }
}

fn commit_history_commit(
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
                visibility: change.visibility,
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
) -> Option<FileChangeKind> {
    match (old_content, new_content) {
        (None, Some(_)) => Some(FileChangeKind::Added),
        (Some(_), Some(_)) => Some(FileChangeKind::Modified),
        (Some(_), None) => Some(FileChangeKind::Deleted),
        (None, None) => None,
    }
}
