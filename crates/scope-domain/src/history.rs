use super::{
    policy::{ScopePath, Visibility},
    projection::{
        ProjectedCommit, Projection, ProjectionViewKey, SourceGraph, VisibilityEvent, project_graph,
    },
    store::{FileChangeKind, LogicalCommitOrigin, SourceBlob},
};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, HashMap};

const HISTORY_GENERATION_VERSION: &str = "v3";

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
    let repo_id = projection.repo_id.clone();
    let view_key = projection.view_key;
    let mut tree = BTreeMap::new();
    let mut entries = if view_key == ProjectionViewKey::Private {
        private_history_entries(
            projection.commits,
            visibility_events,
            &entry_kinds,
            &mut tree,
        )
    } else {
        projection
            .commits
            .into_iter()
            .map(|commit| {
                let kind = entry_kinds
                    .get(commit.logical_commit_id.as_str())
                    .copied()
                    .expect("projected history entry must retain its source kind");
                history_entry(&mut tree, commit, kind)
            })
            .collect::<Vec<_>>()
    };
    let generation = history_generation(&repo_id, view_key, &entries);
    entries.reverse();

    HistoryView {
        repo_id,
        view_key: view_key.as_str().to_string(),
        generation,
        entries,
    }
}

fn private_history_entries(
    commits: Vec<ProjectedCommit>,
    visibility_events: &[VisibilityEvent],
    entry_kinds: &HashMap<String, HistoryEntryKind>,
    tree: &mut BTreeMap<ScopePath, SourceBlob>,
) -> Vec<HistoryEntry> {
    let mut before_graph = Vec::new();
    let mut events_by_anchor: HashMap<&str, Vec<&VisibilityEvent>> = HashMap::new();
    for event in visibility_events
        .iter()
        .filter(|event| event.source_commit_id.is_none())
    {
        match event.after_commit_id.as_deref() {
            Some(anchor) => events_by_anchor.entry(anchor).or_default().push(event),
            None => before_graph.push(event),
        }
    }

    let mut entries = Vec::new();
    let mut parent_id = None;
    append_private_visibility_entries(&mut entries, &mut parent_id, before_graph);
    for commit in commits {
        let source_id = commit.logical_commit_id.clone();
        let kind = entry_kinds
            .get(source_id.as_str())
            .copied()
            .expect("private history entry must retain its source kind");
        let mut entry = history_entry(tree, commit, kind);
        entry.parent_id = parent_id.clone();
        parent_id = Some(entry.id.clone());
        entries.push(entry);
        append_private_visibility_entries(
            &mut entries,
            &mut parent_id,
            events_by_anchor
                .remove(source_id.as_str())
                .unwrap_or_default(),
        );
    }
    entries
}

fn append_private_visibility_entries(
    entries: &mut Vec<HistoryEntry>,
    parent_id: &mut Option<String>,
    events: Vec<&VisibilityEvent>,
) {
    for event in events {
        let id = event.id.clone();
        let files = vec![HistoryEntryFile {
            path: event.path.clone(),
            kind: FileChangeKind::Modified,
            old_content: event.current_content.clone(),
            new_content: event.current_content.clone(),
            visibility: event.new_visibility,
        }];
        entries.push(HistoryEntry {
            id: id.clone(),
            source_id: event.id.clone(),
            parent_id: parent_id.clone(),
            kind: HistoryEntryKind::VisibilityChange,
            author: Some(event.author_id.clone()),
            message: format!(
                "Changed {} visibility to {}",
                event.path.as_str(),
                match event.new_visibility {
                    Visibility::Public => "public",
                    Visibility::Private => "private",
                }
            ),
            files,
        });
        *parent_id = Some(id);
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
    repo_id: &str,
    view_key: ProjectionViewKey,
    entries: &[HistoryEntry],
) -> String {
    let mut hasher = Sha256::new();
    hash_field(
        &mut hasher,
        b"semantics",
        HISTORY_GENERATION_VERSION.as_bytes(),
    );
    hash_field(&mut hasher, b"repo", repo_id.as_bytes());
    hash_field(&mut hasher, b"view", view_key.as_str().as_bytes());
    for entry in entries {
        hash_field(&mut hasher, b"entry", entry.id.as_bytes());
        hash_field(&mut hasher, b"source", entry.source_id.as_bytes());
        hash_field(
            &mut hasher,
            b"kind",
            match entry.kind {
                HistoryEntryKind::Push => b"push",
                HistoryEntryKind::MergedRequest => b"merged_request",
                HistoryEntryKind::VisibilityChange => b"visibility_change",
            },
        );
        hash_optional_field(&mut hasher, b"parent", entry.parent_id.as_deref());
        hash_optional_field(&mut hasher, b"author", entry.author.as_deref());
        hash_field(&mut hasher, b"message", entry.message.as_bytes());
        for file in &entry.files {
            hash_field(&mut hasher, b"path", file.path.as_str().as_bytes());
            hash_field(
                &mut hasher,
                b"visibility",
                match file.visibility {
                    Visibility::Public => b"public",
                    Visibility::Private => b"private",
                },
            );
            hash_optional_blob(&mut hasher, b"old", file.old_content.as_ref());
            hash_optional_blob(&mut hasher, b"new", file.new_content.as_ref());
        }
    }
    hex::encode(hasher.finalize())
}

fn hash_optional_blob(hasher: &mut Sha256, label: &[u8], blob: Option<&SourceBlob>) {
    match blob {
        Some(blob) => {
            hash_field(hasher, label, b"present");
            hash_field(hasher, b"sha256", blob.sha256.as_bytes());
            hash_field(hasher, b"git_oid", blob.git_oid.as_bytes());
            hash_field(hasher, b"mode", blob.git_file_mode.as_bytes());
            hash_field(hasher, b"size", blob.size_bytes.to_string().as_bytes());
        }
        None => hash_field(hasher, label, b"absent"),
    }
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
