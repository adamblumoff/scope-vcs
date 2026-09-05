use super::{
    content::SourceBlob,
    policy::{ScopePath, Visibility},
    projection::{
        LogicalCommitOrigin, ProjectedCommit, Projection, ProjectionViewKey, SourceGraph,
        project_graph,
    },
    visibility_changes::{VisibilityChange, VisibilityChangeSet},
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

const HISTORY_GENERATION_VERSION: &str = "v4";

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum FileChangeKind {
    Added,
    Modified,
    Deleted,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct HistoryView {
    pub repo_id: String,
    pub view_key: String,
    pub generation: String,
    pub entries: Vec<HistoryEntry>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct HistoryEntry {
    pub id: String,
    pub source_id: String,
    pub parent_id: Option<String>,
    pub kind: HistoryEntryKind,
    pub author: Option<String>,
    pub message: String,
    pub files: Vec<HistoryEntryFile>,
    pub visibility_changes: Vec<HistoryEntryVisibilityChange>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum HistoryEntryKind {
    Push,
    MergedRequest,
    VisibilityChange,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct HistoryEntryFile {
    pub path: ScopePath,
    pub kind: FileChangeKind,
    pub old_content: Option<SourceBlob>,
    pub new_content: Option<SourceBlob>,
    pub visibility: Visibility,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct HistoryEntryVisibilityChange {
    pub path: ScopePath,
    pub old_visibility: Visibility,
    pub new_visibility: Visibility,
}

pub fn history_view(
    graph: &SourceGraph,
    visibility_change_sets: &[VisibilityChangeSet],
    view_key: ProjectionViewKey,
) -> HistoryView {
    let projection = project_graph(graph, visibility_change_sets, view_key);
    history_view_from_projection(projection, graph, visibility_change_sets)
}

pub fn history_view_from_projection(
    projection: Projection,
    graph: &SourceGraph,
    visibility_change_sets: &[VisibilityChangeSet],
) -> HistoryView {
    let logical_source_ids = graph
        .commits
        .iter()
        .map(|commit| commit.id.as_str())
        .collect::<HashSet<_>>();
    let entry_kinds = history_entry_kinds(graph, visibility_change_sets, &logical_source_ids);
    let visibility_sets_by_source =
        visibility_sets_by_source(visibility_change_sets, &logical_source_ids);
    let repo_id = projection.repo_id.clone();
    let view_key = projection.view_key;
    let mut tree = BTreeMap::new();
    let mut entries = if view_key == ProjectionViewKey::Private {
        private_history_entries(
            projection.commits,
            visibility_change_sets,
            &logical_source_ids,
            &entry_kinds,
            &mut tree,
        )
    } else {
        public_history_entries(
            projection.commits,
            &visibility_sets_by_source,
            &entry_kinds,
            &mut tree,
        )
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

fn public_history_entries(
    commits: Vec<ProjectedCommit>,
    visibility_sets_by_source: &HashMap<&str, Vec<&VisibilityChangeSet>>,
    entry_kinds: &HashMap<String, HistoryEntryKind>,
    tree: &mut BTreeMap<ScopePath, SourceBlob>,
) -> Vec<HistoryEntry> {
    let mut entries = Vec::<HistoryEntry>::new();
    for commit in commits {
        let source_id = commit.logical_commit_id.clone();
        let kind = entry_kinds
            .get(source_id.as_str())
            .copied()
            .unwrap_or(HistoryEntryKind::VisibilityChange);
        let fragment = history_entry(tree, commit, kind);
        if let Some(previous) = entries
            .last_mut()
            .filter(|previous| previous.source_id == fragment.source_id)
        {
            previous.files.extend(fragment.files);
            if fragment.author.is_some() {
                previous.author = fragment.author;
            }
            if !matches!(
                fragment.message.as_str(),
                "Projection baseline" | "Projection visibility boundary"
            ) {
                previous.message = fragment.message;
            }
        } else {
            entries.push(fragment);
        }
    }

    for entry in &mut entries {
        let visible_paths = entry
            .files
            .iter()
            .map(|file| &file.path)
            .collect::<BTreeSet<_>>();
        entry.visibility_changes = visibility_sets_by_source
            .get(entry.source_id.as_str())
            .into_iter()
            .flat_map(|sets| sets.iter())
            .flat_map(|set| &set.changes)
            .filter(|change| visible_paths.contains(&change.path))
            .map(history_visibility_change)
            .collect();
        if entry.kind == HistoryEntryKind::VisibilityChange {
            entry.message = visibility_change_message(&entry.visibility_changes);
        }
    }
    relink_semantic_parents(&mut entries);
    entries
}

fn private_history_entries(
    commits: Vec<ProjectedCommit>,
    visibility_change_sets: &[VisibilityChangeSet],
    logical_source_ids: &HashSet<&str>,
    entry_kinds: &HashMap<String, HistoryEntryKind>,
    tree: &mut BTreeMap<ScopePath, SourceBlob>,
) -> Vec<HistoryEntry> {
    let mut before_graph = Vec::new();
    let mut sets_by_anchor: HashMap<&str, Vec<&VisibilityChangeSet>> = HashMap::new();
    let mut sets_by_source: HashMap<&str, Vec<&VisibilityChangeSet>> = HashMap::new();
    for set in visibility_change_sets {
        let source_id = resolved_set_source_id(set, logical_source_ids);
        if source_id != set.id {
            sets_by_source.entry(source_id).or_default().push(set);
            continue;
        }
        match set
            .anchor_commit_id
            .as_deref()
            .filter(|anchor| logical_source_ids.contains(anchor))
        {
            Some(anchor) => sets_by_anchor.entry(anchor).or_default().push(set),
            None => before_graph.push(set),
        }
    }

    let mut entries = Vec::new();
    append_private_visibility_entries(&mut entries, before_graph);
    for commit in commits {
        let source_id = commit.logical_commit_id.clone();
        let kind = entry_kinds
            .get(source_id.as_str())
            .copied()
            .expect("private history entry must retain its source kind");
        let mut entry = history_entry(tree, commit, kind);
        entry.visibility_changes = sets_by_source
            .remove(source_id.as_str())
            .into_iter()
            .flatten()
            .flat_map(|set| &set.changes)
            .map(history_visibility_change)
            .collect();
        entries.push(entry);
        append_private_visibility_entries(
            &mut entries,
            sets_by_anchor
                .remove(source_id.as_str())
                .unwrap_or_default(),
        );
    }
    relink_semantic_parents(&mut entries);
    entries
}

fn append_private_visibility_entries(
    entries: &mut Vec<HistoryEntry>,
    sets: Vec<&VisibilityChangeSet>,
) {
    entries.extend(sets.into_iter().map(|set| {
        let visibility_changes = set
            .changes
            .iter()
            .map(history_visibility_change)
            .collect::<Vec<_>>();
        HistoryEntry {
            id: set.id.clone(),
            source_id: set.id.clone(),
            parent_id: None,
            kind: HistoryEntryKind::VisibilityChange,
            author: Some(set.author_id.clone()),
            message: visibility_change_message(&visibility_changes),
            files: Vec::new(),
            visibility_changes,
        }
    }));
}

fn relink_semantic_parents(entries: &mut [HistoryEntry]) {
    let mut parent_id = None;
    for entry in entries {
        entry.parent_id = parent_id;
        parent_id = Some(entry.source_id.clone());
    }
}

fn visibility_sets_by_source<'a>(
    sets: &'a [VisibilityChangeSet],
    logical_source_ids: &HashSet<&str>,
) -> HashMap<&'a str, Vec<&'a VisibilityChangeSet>> {
    let mut sets_by_source = HashMap::<&str, Vec<&VisibilityChangeSet>>::new();
    for set in sets {
        sets_by_source
            .entry(resolved_set_source_id(set, logical_source_ids))
            .or_default()
            .push(set);
    }
    sets_by_source
}

fn resolved_set_source_id<'a>(
    set: &'a VisibilityChangeSet,
    logical_source_ids: &HashSet<&str>,
) -> &'a str {
    set.source_update_id
        .as_deref()
        .filter(|source_id| logical_source_ids.contains(source_id))
        .unwrap_or(&set.id)
}

fn history_visibility_change(change: &VisibilityChange) -> HistoryEntryVisibilityChange {
    HistoryEntryVisibilityChange {
        path: change.path.clone(),
        old_visibility: change.old_visibility,
        new_visibility: change.new_visibility,
    }
}

fn visibility_change_message(changes: &[HistoryEntryVisibilityChange]) -> String {
    let made_public = changes
        .iter()
        .filter(|change| change.new_visibility == Visibility::Public)
        .count();
    let made_private = changes.len().saturating_sub(made_public);
    match (made_public, made_private) {
        (public, 0) => format!("Made {public} {} public", file_word(public)),
        (0, private) => format!("Made {private} {} private", file_word(private)),
        _ => format!("Updated visibility for {} files", changes.len()),
    }
}

fn file_word(count: usize) -> &'static str {
    if count == 1 { "file" } else { "files" }
}

fn history_entry_kinds(
    graph: &SourceGraph,
    visibility_change_sets: &[VisibilityChangeSet],
    logical_source_ids: &HashSet<&str>,
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
    for set in visibility_change_sets {
        if resolved_set_source_id(set, logical_source_ids) == set.id {
            kinds.insert(set.id.clone(), HistoryEntryKind::VisibilityChange);
        }
    }
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
                visibility_bytes(file.visibility),
            );
            hash_optional_blob(&mut hasher, b"old", file.old_content.as_ref());
            hash_optional_blob(&mut hasher, b"new", file.new_content.as_ref());
        }
        for change in &entry.visibility_changes {
            hash_field(
                &mut hasher,
                b"visibility_path",
                change.path.as_str().as_bytes(),
            );
            hash_field(
                &mut hasher,
                b"old_visibility",
                visibility_bytes(change.old_visibility),
            );
            hash_field(
                &mut hasher,
                b"new_visibility",
                visibility_bytes(change.new_visibility),
            );
        }
    }
    hex::encode(hasher.finalize())
}

fn visibility_bytes(visibility: Visibility) -> &'static [u8] {
    match visibility {
        Visibility::Public => b"public",
        Visibility::Private => b"private",
    }
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
        id: commit.logical_commit_id.clone(),
        source_id: commit.logical_commit_id,
        parent_id: None,
        kind,
        author: commit.author,
        message: commit.message,
        files,
        visibility_changes: Vec::new(),
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
