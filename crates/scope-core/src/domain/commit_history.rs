use super::{
    policy::{Policy, ScopePath, Visibility},
    projection::{ProjectedCommit, Projection, ProjectionViewKey, VisibilityEvent, project_graph},
    store::{FileChangeKind, SourceBlob},
};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;

const COMMIT_HISTORY_GENERATION_VERSION: &str = "v1";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CommitHistoryView {
    pub repo_id: String,
    pub view_key: String,
    pub generation: String,
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
    let generation = commit_history_generation(&projection);
    let mut tree = BTreeMap::new();
    let commits = projection
        .commits
        .into_iter()
        .map(|commit| commit_history_commit(&mut tree, commit))
        .collect();

    CommitHistoryView {
        repo_id: graph.repo_id.clone(),
        view_key: projection.view_key.as_str().to_string(),
        generation,
        commits,
    }
}

fn commit_history_generation(projection: &Projection) -> String {
    let mut hasher = Sha256::new();
    hash_field(
        &mut hasher,
        b"semantics",
        COMMIT_HISTORY_GENERATION_VERSION.as_bytes(),
    );
    hash_field(&mut hasher, b"repo", projection.repo_id.as_bytes());
    hash_field(
        &mut hasher,
        b"view",
        projection.view_key.as_str().as_bytes(),
    );
    for commit in &projection.commits {
        hash_field(&mut hasher, b"commit", commit.projected_id.as_bytes());
        hash_field(&mut hasher, b"logical", commit.logical_commit_id.as_bytes());
        hash_optional_field(
            &mut hasher,
            b"parent",
            commit.parent_projected_id.as_deref(),
        );
        hash_optional_field(&mut hasher, b"author", commit.author.as_deref());
        hash_field(&mut hasher, b"message", commit.message.as_bytes());
        for change in &commit.changes {
            hash_field(&mut hasher, b"path", change.path.as_str().as_bytes());
            hash_field(
                &mut hasher,
                b"visibility",
                match change.visibility {
                    Visibility::Public => b"public",
                    Visibility::Private => b"private",
                },
            );
            match &change.new_content {
                Some(blob) => {
                    hash_field(&mut hasher, b"sha256", blob.sha256.as_bytes());
                    hash_field(&mut hasher, b"git_oid", blob.git_oid.as_bytes());
                    hash_field(&mut hasher, b"mode", blob.git_file_mode.as_bytes());
                    hash_field(&mut hasher, b"size", blob.size_bytes.to_string().as_bytes());
                }
                None => hash_field(&mut hasher, b"delete", b""),
            }
        }
    }
    hex::encode(hasher.finalize())
}

fn hash_optional_field(hasher: &mut Sha256, label: &[u8], value: Option<&str>) {
    match value {
        Some(value) => {
            hash_field(hasher, label, b"present");
            hash_field(hasher, label, value.as_bytes());
        }
        None => hash_field(hasher, label, b"absent"),
    }
}

fn hash_field(hasher: &mut Sha256, label: &[u8], value: &[u8]) {
    hasher.update((label.len() as u64).to_be_bytes());
    hasher.update(label);
    hasher.update((value.len() as u64).to_be_bytes());
    hasher.update(value);
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
