use super::{
    policy::{ScopePath, Visibility},
    repo_control::{is_private_control_path, is_repo_control_path},
    store::{LogicalCommitOrigin, RepositoryAccess, RepositoryActor, SourceBlob},
    visibility_changes::{VisibilityChange, VisibilityChangeSet},
};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileChange {
    pub path: ScopePath,
    pub old_content: Option<SourceBlob>,
    pub new_content: Option<SourceBlob>,
    pub visibility: Visibility,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LogicalCommit {
    pub id: String,
    pub origin: LogicalCommitOrigin,
    pub author_id: String,
    pub message: String,
    pub changes: Vec<FileChange>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SourceGraph {
    pub repo_id: String,
    pub commits: Vec<LogicalCommit>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProjectedChange {
    pub path: ScopePath,
    pub new_content: Option<SourceBlob>,
    pub visibility: Visibility,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProjectedCommit {
    pub projected_id: String,
    pub logical_commit_id: String,
    pub parent_projected_id: Option<String>,
    pub author: Option<String>,
    pub message: String,
    pub changes: Vec<ProjectedChange>,
    pub materialization: ProjectionMaterialization,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ProjectionMaterialization {
    Generate,
    PreserveGitCommit {
        oid: String,
        parent_oids: Vec<String>,
        tree_oid: String,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ProjectionViewKey {
    Private,
    Public,
}

impl ProjectionViewKey {
    pub fn from_access(access: RepositoryAccess) -> Self {
        match access.actor {
            RepositoryActor::Owner => Self::Private,
            RepositoryActor::Member if access.can_read_private_files => Self::Private,
            RepositoryActor::Member | RepositoryActor::Public => Self::Public,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Private => "private",
            Self::Public => "public",
        }
    }

    fn can_read_private_files(self) -> bool {
        matches!(self, Self::Private)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Projection {
    pub repo_id: String,
    pub view_key: ProjectionViewKey,
    pub commits: Vec<ProjectedCommit>,
}

impl Projection {
    pub fn preserves_git_commits(&self) -> bool {
        self.commits.iter().any(|commit| {
            matches!(
                &commit.materialization,
                ProjectionMaterialization::PreserveGitCommit { .. }
            )
        })
    }

    pub fn visible_paths(&self) -> Vec<String> {
        let mut live = BTreeMap::new();
        for change in self.commits.iter().flat_map(|commit| commit.changes.iter()) {
            if change.new_content.is_some() {
                live.insert(change.path.as_str().to_string(), ());
            } else {
                live.remove(change.path.as_str());
            }
        }
        live.into_keys().collect::<Vec<_>>()
    }
}

pub fn project_graph(
    graph: &SourceGraph,
    visibility_change_sets: &[VisibilityChangeSet],
    view_key: ProjectionViewKey,
) -> Projection {
    if view_key.can_read_private_files() {
        return project_private_graph(graph, view_key);
    }

    let mut commits = Vec::new();
    let mut last_visible: Option<String> = None;
    let boundary_events = projection_boundary_events_by_anchor(graph, visibility_change_sets);

    process_projection_boundary_events_after(
        &mut commits,
        &mut last_visible,
        &boundary_events,
        None,
        view_key,
    );

    for logical in &graph.commits {
        let mut visible_changes = logical
            .changes
            .iter()
            .filter(|change| {
                change.visibility == Visibility::Public && !is_private_control_path(&change.path)
            })
            .map(|change| ProjectedChange {
                path: change.path.clone(),
                new_content: change.new_content.clone(),
                visibility: change.visibility,
            })
            .collect::<Vec<_>>();
        let visible_content_count = visible_changes.len();

        if let LogicalCommitOrigin::PublicRequestMerge {
            commits: native,
            preserve_public_commits: true,
            ..
        } = &logical.origin
            && !native.is_empty()
            && visible_content_count == logical.changes.len()
        {
            let native_len = native.len();
            for (index, native) in native.iter().enumerate() {
                let is_head = index + 1 == native_len;
                commits.push(ProjectedCommit {
                    projected_id: native.oid.clone(),
                    logical_commit_id: logical.id.clone(),
                    parent_projected_id: native.parent_oids.first().cloned(),
                    author: None,
                    message: if is_head {
                        logical.message.clone()
                    } else {
                        "Preserved public request commit".to_string()
                    },
                    changes: if is_head {
                        std::mem::take(&mut visible_changes)
                    } else {
                        Vec::new()
                    },
                    materialization: ProjectionMaterialization::PreserveGitCommit {
                        oid: native.oid.clone(),
                        parent_oids: native.parent_oids.clone(),
                        tree_oid: native.tree_oid.clone(),
                    },
                });
            }
            last_visible = native.last().map(|commit| commit.oid.clone());
            process_projection_boundary_events_after(
                &mut commits,
                &mut last_visible,
                &boundary_events,
                Some(logical.id.as_str()),
                view_key,
            );
            continue;
        }

        if visible_changes.is_empty() {
            process_projection_boundary_events_after(
                &mut commits,
                &mut last_visible,
                &boundary_events,
                Some(logical.id.as_str()),
                view_key,
            );
            continue;
        }

        let partial = visible_content_count < logical.changes.len();
        let projected_id = projected_id(view_key, &logical.id, commits.len() + 1);

        commits.push(ProjectedCommit {
            projected_id: projected_id.clone(),
            logical_commit_id: logical.id.clone(),
            parent_projected_id: last_visible,
            author: (!partial).then(|| logical.author_id.clone()),
            message: if partial {
                "Projected public update".to_string()
            } else {
                logical.message.clone()
            },
            changes: visible_changes,
            materialization: ProjectionMaterialization::Generate,
        });

        last_visible = Some(projected_id);
        process_projection_boundary_events_after(
            &mut commits,
            &mut last_visible,
            &boundary_events,
            Some(logical.id.as_str()),
            view_key,
        );
    }

    Projection {
        repo_id: graph.repo_id.clone(),
        view_key,
        commits,
    }
}

fn project_private_graph(graph: &SourceGraph, view_key: ProjectionViewKey) -> Projection {
    let mut commits = Vec::new();
    let mut last_visible: Option<String> = None;

    for logical in &graph.commits {
        let changes = logical
            .changes
            .iter()
            .map(|change| ProjectedChange {
                path: change.path.clone(),
                new_content: change.new_content.clone(),
                visibility: change.visibility,
            })
            .collect::<Vec<_>>();
        if changes.is_empty() {
            continue;
        }

        let projected_id = projected_id(view_key, &logical.id, commits.len() + 1);
        commits.push(ProjectedCommit {
            projected_id: projected_id.clone(),
            logical_commit_id: logical.id.clone(),
            parent_projected_id: last_visible,
            author: Some(logical.author_id.clone()),
            message: logical.message.clone(),
            changes,
            materialization: ProjectionMaterialization::Generate,
        });
        last_visible = Some(projected_id);
    }

    Projection {
        repo_id: graph.repo_id.clone(),
        view_key,
        commits,
    }
}

fn projected_id(view_key: ProjectionViewKey, source_id: &str, sequence: usize) -> String {
    format!("pv_{}_{}_{}", view_key.as_str(), source_id, sequence)
}

struct ProjectionBoundaryEventsByAnchor<'a> {
    before_graph: Vec<ProjectionBoundaryEvent<'a>>,
    after_commits: BTreeMap<&'a str, Vec<ProjectionBoundaryEvent<'a>>>,
}

#[derive(Clone, Copy)]
struct ProjectionBoundaryEvent<'a> {
    set: &'a VisibilityChangeSet,
    change: &'a VisibilityChange,
    new_content: Option<&'a SourceBlob>,
    source_id: &'a str,
    source_update_resolved: bool,
}

fn projection_boundary_events_by_anchor<'a>(
    graph: &'a SourceGraph,
    sets: &'a [VisibilityChangeSet],
) -> ProjectionBoundaryEventsByAnchor<'a> {
    let commits_by_id = graph
        .commits
        .iter()
        .map(|commit| (commit.id.as_str(), commit))
        .collect::<HashMap<_, _>>();
    let mut events_by_anchor = ProjectionBoundaryEventsByAnchor {
        before_graph: Vec::new(),
        after_commits: BTreeMap::new(),
    };
    for set in sets {
        let source_update = set
            .source_update_id
            .as_deref()
            .and_then(|source_id| commits_by_id.get(source_id).copied());
        let source_id = source_update.map_or(set.id.as_str(), |commit| commit.id.as_str());
        for change in set
            .changes
            .iter()
            .filter(|change| !is_repo_control_path(&change.path))
        {
            let boundary = match (change.old_visibility, change.new_visibility) {
                (Visibility::Private, Visibility::Public)
                    if !source_update.is_some_and(|commit| {
                        commit
                            .changes
                            .iter()
                            .any(|source_change| source_change.path == change.path)
                    }) =>
                {
                    let Some(content) = change.current_content.as_ref() else {
                        continue;
                    };
                    ProjectionBoundaryEvent {
                        set,
                        change,
                        new_content: Some(content),
                        source_id,
                        source_update_resolved: source_update.is_some(),
                    }
                }
                (Visibility::Public, Visibility::Private) => ProjectionBoundaryEvent {
                    set,
                    change,
                    new_content: None,
                    source_id,
                    source_update_resolved: source_update.is_some(),
                },
                _ => continue,
            };
            match set
                .anchor_commit_id
                .as_deref()
                .filter(|anchor| commits_by_id.contains_key(anchor))
            {
                Some(after_commit_id) => events_by_anchor
                    .after_commits
                    .entry(after_commit_id)
                    .or_default()
                    .push(boundary),
                None => events_by_anchor.before_graph.push(boundary),
            }
        }
    }
    events_by_anchor
}

fn process_projection_boundary_events_after(
    commits: &mut Vec<ProjectedCommit>,
    last_visible: &mut Option<String>,
    boundary_events: &ProjectionBoundaryEventsByAnchor<'_>,
    after_commit_id: Option<&str>,
    view_key: ProjectionViewKey,
) {
    let events = match after_commit_id {
        Some(after_commit_id) => boundary_events
            .after_commits
            .get(after_commit_id)
            .map(Vec::as_slice)
            .unwrap_or(&[]),
        None => boundary_events.before_graph.as_slice(),
    };

    for (set_id, boundaries) in group_boundary_events_by_set(events) {
        let logical_commit_id = boundaries[0].source_id.to_string();
        let projected_id = projected_id(view_key, set_id, commits.len() + 1);
        commits.push(ProjectedCommit {
            projected_id: projected_id.clone(),
            logical_commit_id,
            parent_projected_id: last_visible.clone(),
            author: None,
            message: if boundaries[0].source_update_resolved {
                "Projected public update".to_string()
            } else if boundaries
                .iter()
                .all(|boundary| boundary.new_content.is_some())
            {
                "Projection baseline".to_string()
            } else {
                "Projection visibility boundary".to_string()
            },
            changes: boundaries
                .into_iter()
                .map(|boundary| ProjectedChange {
                    path: boundary.change.path.clone(),
                    new_content: boundary.new_content.cloned(),
                    visibility: if boundary.new_content.is_some() {
                        boundary.change.new_visibility
                    } else {
                        boundary.change.old_visibility
                    },
                })
                .collect(),
            materialization: ProjectionMaterialization::Generate,
        });
        *last_visible = Some(projected_id);
    }
}

fn group_boundary_events_by_set<'a>(
    events: &'a [ProjectionBoundaryEvent<'a>],
) -> Vec<(&'a str, Vec<ProjectionBoundaryEvent<'a>>)> {
    let mut groups = Vec::<(&str, Vec<ProjectionBoundaryEvent<'_>>)>::new();
    for event in events {
        if let Some((_, boundaries)) = groups
            .iter_mut()
            .find(|(set_id, _)| *set_id == event.set.id)
        {
            boundaries.push(*event);
        } else {
            groups.push((event.set.id.as_str(), vec![*event]));
        }
    }
    groups
}
