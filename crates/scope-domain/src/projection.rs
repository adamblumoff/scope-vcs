use super::{
    policy::{Policy, ScopePath, Visibility},
    repo_config::is_reserved_config_path,
    store::{RepositoryAccess, RepositoryActor, SourceBlob},
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum AuthorVisibility {
    Visible,
    Hidden,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileChange {
    pub path: ScopePath,
    pub old_content: Option<SourceBlob>,
    pub new_content: Option<SourceBlob>,
    pub visibility: Visibility,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct VisibilityEvent {
    pub id: String,
    pub after_commit_id: Option<String>,
    pub source_commit_id: Option<String>,
    pub author_id: String,
    pub path: ScopePath,
    pub old_visibility: Visibility,
    pub new_visibility: Visibility,
    pub current_content: Option<SourceBlob>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LogicalCommit {
    pub id: String,
    pub parent_ids: Vec<String>,
    pub author_id: String,
    pub author_visibility: AuthorVisibility,
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
    _policy: &Policy,
    graph: &SourceGraph,
    visibility_events: &[VisibilityEvent],
    view_key: ProjectionViewKey,
) -> Projection {
    if view_key.can_read_private_files() {
        return project_private_graph(graph, view_key);
    }

    let mut commits = Vec::new();
    let mut last_visible: Option<String> = None;
    let boundary_events = projection_boundary_events_by_anchor(visibility_events);

    process_projection_boundary_events_after(
        &mut commits,
        &mut last_visible,
        &boundary_events,
        None,
        view_key,
    );

    for logical in &graph.commits {
        let visible_changes = logical
            .changes
            .iter()
            .filter(|change| {
                change.visibility == Visibility::Public && !is_reserved_config_path(&change.path)
            })
            .map(|change| ProjectedChange {
                path: change.path.clone(),
                new_content: change.new_content.clone(),
                visibility: change.visibility,
            })
            .collect::<Vec<_>>();
        let visible_content_count = visible_changes.len();

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
            author: if partial {
                None
            } else {
                match logical.author_visibility {
                    AuthorVisibility::Visible => Some(logical.author_id.clone()),
                    AuthorVisibility::Hidden => None,
                }
            },
            message: if partial {
                "Projected public update".to_string()
            } else {
                logical.message.clone()
            },
            changes: visible_changes,
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
            author: match logical.author_visibility {
                AuthorVisibility::Visible => Some(logical.author_id.clone()),
                AuthorVisibility::Hidden => None,
            },
            message: logical.message.clone(),
            changes,
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
    event: &'a VisibilityEvent,
    new_content: Option<&'a SourceBlob>,
}

fn projection_boundary_events_by_anchor<'a>(
    events: &'a [VisibilityEvent],
) -> ProjectionBoundaryEventsByAnchor<'a> {
    let mut events_by_anchor = ProjectionBoundaryEventsByAnchor {
        before_graph: Vec::new(),
        after_commits: BTreeMap::new(),
    };
    for event in events
        .iter()
        .filter(|event| !is_reserved_config_path(&event.path))
    {
        let boundary = match (event.old_visibility, event.new_visibility) {
            (Visibility::Private, Visibility::Public) if event.source_commit_id.is_none() => {
                let Some(content) = event.current_content.as_ref() else {
                    continue;
                };
                ProjectionBoundaryEvent {
                    event,
                    new_content: Some(content),
                }
            }
            (Visibility::Public, Visibility::Private) => ProjectionBoundaryEvent {
                event,
                new_content: None,
            },
            _ => continue,
        };
        match event.after_commit_id.as_deref() {
            Some(after_commit_id) => events_by_anchor
                .after_commits
                .entry(after_commit_id)
                .or_default()
                .push(boundary),
            None => events_by_anchor.before_graph.push(boundary),
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

    for boundary in events {
        let event = boundary.event;
        let projected_id = projected_id(view_key, &event.id, commits.len() + 1);
        commits.push(ProjectedCommit {
            projected_id: projected_id.clone(),
            logical_commit_id: event.id.clone(),
            parent_projected_id: last_visible.clone(),
            author: None,
            message: if boundary.new_content.is_some() {
                "Projection baseline".to_string()
            } else {
                "Projection visibility boundary".to_string()
            },
            changes: vec![ProjectedChange {
                path: event.path.clone(),
                new_content: boundary.new_content.cloned(),
                visibility: if boundary.new_content.is_some() {
                    event.new_visibility
                } else {
                    event.old_visibility
                },
            }],
        });
        *last_visible = Some(projected_id);
    }
}
