use super::{
    policy::{ScopePath, Visibility},
    projection::{
        ProjectedCommit, Projection, ProjectionViewKey, SourceGraph, VisibilityEvent, project_graph,
    },
    store::{FileChangeKind, LogicalCommitOrigin, SourceBlob},
};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, HashMap};

const HISTORY_GENERATION_VERSION: &str = "v2";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HistoryView {
    pub repo_id: String,
    pub view_key: String,
    pub generation: String,
    pub entries: Vec<HistoryEntry>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HistoryEntry {
    pub id: String,
    pub source_id: String,
    pub parent_id: Option<String>,
    pub kind: HistoryEntryKind,
    pub author: Option<String>,
    pub message: String,
    pub files: Vec<HistoryEntryFile>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HistoryEntryKind {
    Push,
    MergedRequest,
    VisibilityChange,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HistoryEntryFile {
    pub path: ScopePath,
    pub kind: FileChangeKind,
    pub old_content: Option<SourceBlob>,
    pub new_content: Option<SourceBlob>,
    pub visibility: Visibility,
}

pub fn history_view(
    graph: &SourceGraph,
    visibility_events: &[VisibilityEvent],
    view_key: ProjectionViewKey,
) -> HistoryView {
    let projection = project_graph(graph, visibility_events, view_key);
    history_view_from_projection(projection, graph, visibility_events)
}

pub fn history_view_from_projection(
    projection: Projection,
    graph: &SourceGraph,
    visibility_events: &[VisibilityEvent],
) -> HistoryView {
    let entry_kinds = history_entry_kinds(graph, visibility_events);
    let generation = history_generation(&projection, &entry_kinds);
    let mut tree = BTreeMap::new();
    let mut entries = projection
        .commits
        .into_iter()
        .map(|commit| {
            let kind = entry_kinds
                .get(commit.logical_commit_id.as_str())
                .copied()
                .expect("projected history entry must retain its source kind");
            history_entry(&mut tree, commit, kind)
        })
        .collect::<Vec<_>>();
    entries.reverse();

    HistoryView {
        repo_id: projection.repo_id.clone(),
        view_key: projection.view_key.as_str().to_string(),
        generation,
        entries,
    }
}

fn history_entry_kinds(
    graph: &SourceGraph,
    visibility_events: &[VisibilityEvent],
) -> HashMap<String, HistoryEntryKind> {
    let mut kinds = graph
        .commits
        .iter()
        .map(|commit| {
            let kind = match commit.origin {
                LogicalCommitOrigin::CanonicalPush { .. } => HistoryEntryKind::Push,
                LogicalCommitOrigin::PrivateRequestMerge { .. }
                | LogicalCommitOrigin::PublicRequestMerge { .. } => HistoryEntryKind::MergedRequest,
            };
            (commit.id.clone(), kind)
        })
        .collect::<HashMap<_, _>>();
    kinds.extend(
        visibility_events
            .iter()
            .map(|event| (event.id.clone(), HistoryEntryKind::VisibilityChange)),
    );
    kinds
}

fn history_generation(
    projection: &Projection,
    entry_kinds: &HashMap<String, HistoryEntryKind>,
) -> String {
    let mut hasher = Sha256::new();
    hash_field(
        &mut hasher,
        b"semantics",
        HISTORY_GENERATION_VERSION.as_bytes(),
    );
    hash_field(&mut hasher, b"repo", projection.repo_id.as_bytes());
    hash_field(
        &mut hasher,
        b"view",
        projection.view_key.as_str().as_bytes(),
    );
    for commit in &projection.commits {
        hash_field(&mut hasher, b"entry", commit.projected_id.as_bytes());
        hash_field(&mut hasher, b"source", commit.logical_commit_id.as_bytes());
        let kind = entry_kinds
            .get(commit.logical_commit_id.as_str())
            .expect("projected history entry must retain its source kind");
        hash_field(
            &mut hasher,
            b"kind",
            match kind {
                HistoryEntryKind::Push => b"push",
                HistoryEntryKind::MergedRequest => b"merged_request",
                HistoryEntryKind::VisibilityChange => b"visibility_change",
            },
        );
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

fn history_entry(
    tree: &mut BTreeMap<ScopePath, SourceBlob>,
    commit: ProjectedCommit,
    kind: HistoryEntryKind,
) -> HistoryEntry {
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

            Some(HistoryEntryFile {
                visibility: change.visibility,
                path: change.path,
                kind,
                old_content,
                new_content,
            })
        })
        .collect();

    HistoryEntry {
        id: commit.projected_id,
        source_id: commit.logical_commit_id,
        parent_id: commit.parent_projected_id,
        kind,
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
