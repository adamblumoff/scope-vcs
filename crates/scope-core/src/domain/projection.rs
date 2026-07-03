use super::{
    policy::{Policy, ScopePath, Visibility},
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
        let mut paths = self
            .commits
            .iter()
            .flat_map(|commit| commit.changes.iter())
            .map(|change| change.path.as_str().to_string())
            .collect::<Vec<_>>();
        paths.sort();
        paths.dedup();
        paths
    }
}

pub fn project_graph(
    policy: &Policy,
    graph: &SourceGraph,
    visibility_events: &[VisibilityEvent],
    view_key: ProjectionViewKey,
) -> Projection {
    if view_key.can_read_private_files() {
        return project_private_graph(graph, view_key);
    }

    let mut commits = Vec::new();
    let mut last_visible: Option<String> = None;
    let commit_positions = graph
        .commits
        .iter()
        .enumerate()
        .map(|(index, commit)| (commit.id.as_str(), index))
        .collect::<BTreeMap<_, _>>();
    let readable_epoch_starts = readable_epoch_starts(policy, visibility_events, &commit_positions);
    let baseline_events =
        baseline_events_by_anchor(policy, visibility_events, &readable_epoch_starts);

    process_baseline_events_after(
        &mut commits,
        &mut last_visible,
        &baseline_events,
        None,
        view_key,
    );

    for (commit_index, logical) in graph.commits.iter().enumerate() {
        let visible_changes = logical
            .changes
            .iter()
            .filter(|change| {
                can_read_historical_change(policy, &change.path, change.visibility)
                    && is_change_in_readable_epoch(
                        &readable_epoch_starts,
                        &change.path,
                        commit_index,
                    )
            })
            .map(|change| ProjectedChange {
                path: change.path.clone(),
                new_content: change.new_content.clone(),
            })
            .collect::<Vec<_>>();
        let visible_content_count = visible_changes.len();

        if visible_changes.is_empty() {
            process_baseline_events_after(
                &mut commits,
                &mut last_visible,
                &baseline_events,
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
        process_baseline_events_after(
            &mut commits,
            &mut last_visible,
            &baseline_events,
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

fn can_read_historical_change(policy: &Policy, path: &ScopePath, visibility: Visibility) -> bool {
    match visibility {
        Visibility::Public => policy.can_read(path, false),
        Visibility::Private => false,
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct ReadableEpochStart {
    source_commit_index: Option<usize>,
    baseline_event_id: Option<String>,
}

fn readable_epoch_starts(
    policy: &Policy,
    events: &[VisibilityEvent],
    commit_positions: &BTreeMap<&str, usize>,
) -> BTreeMap<ScopePath, ReadableEpochStart> {
    let mut starts = BTreeMap::new();
    for event in events {
        if !policy.can_read(&event.path, false) {
            continue;
        }

        let old_readable = visibility_readable_for_event(policy, &event.path, event.old_visibility);
        let new_readable = visibility_readable_for_event(policy, &event.path, event.new_visibility);
        if old_readable || !new_readable {
            continue;
        }

        let source_commit_index = event
            .source_commit_id
            .as_deref()
            .and_then(|id| commit_positions.get(id).copied())
            .or_else(|| {
                event
                    .after_commit_id
                    .as_deref()
                    .and_then(|id| commit_positions.get(id).map(|index| index + 1))
            })
            .or(Some(0));
        starts.insert(
            event.path.clone(),
            ReadableEpochStart {
                source_commit_index,
                baseline_event_id: event.source_commit_id.is_none().then(|| event.id.clone()),
            },
        );
    }
    starts
}

fn visibility_readable_for_event(
    policy: &Policy,
    path: &ScopePath,
    visibility: Visibility,
) -> bool {
    match visibility {
        Visibility::Public => true,
        Visibility::Private => {
            policy.effective_visibility(path) == Visibility::Private && policy.can_read(path, false)
        }
    }
}

fn is_change_in_readable_epoch(
    starts: &BTreeMap<ScopePath, ReadableEpochStart>,
    path: &ScopePath,
    commit_index: usize,
) -> bool {
    starts
        .get(path)
        .and_then(|start| start.source_commit_index)
        .is_none_or(|start_index| commit_index >= start_index)
}

struct BaselineEventsByAnchor<'a> {
    before_graph: Vec<&'a VisibilityEvent>,
    after_commits: BTreeMap<&'a str, Vec<&'a VisibilityEvent>>,
}

fn baseline_events_by_anchor<'a>(
    policy: &Policy,
    events: &'a [VisibilityEvent],
    starts: &BTreeMap<ScopePath, ReadableEpochStart>,
) -> BaselineEventsByAnchor<'a> {
    let mut events_by_anchor = BaselineEventsByAnchor {
        before_graph: Vec::new(),
        after_commits: BTreeMap::new(),
    };
    for event in events.iter().filter(|event| {
        starts
            .get(&event.path)
            .and_then(|start| start.baseline_event_id.as_deref())
            == Some(event.id.as_str())
            && policy.can_read(&event.path, false)
            && event.current_content.is_some()
    }) {
        match event.after_commit_id.as_deref() {
            Some(after_commit_id) => events_by_anchor
                .after_commits
                .entry(after_commit_id)
                .or_default()
                .push(event),
            None => events_by_anchor.before_graph.push(event),
        }
    }
    events_by_anchor
}

fn process_baseline_events_after(
    commits: &mut Vec<ProjectedCommit>,
    last_visible: &mut Option<String>,
    baseline_events: &BaselineEventsByAnchor<'_>,
    after_commit_id: Option<&str>,
    view_key: ProjectionViewKey,
) {
    let events = match after_commit_id {
        Some(after_commit_id) => baseline_events
            .after_commits
            .get(after_commit_id)
            .map(Vec::as_slice)
            .unwrap_or(&[]),
        None => baseline_events.before_graph.as_slice(),
    };

    for event in events {
        let projected_id = projected_id(view_key, &event.id, commits.len() + 1);
        commits.push(ProjectedCommit {
            projected_id: projected_id.clone(),
            logical_commit_id: event.id.clone(),
            parent_projected_id: last_visible.clone(),
            author: None,
            message: "Projection baseline".to_string(),
            changes: vec![ProjectedChange {
                path: event.path.clone(),
                new_content: event.current_content.clone(),
            }],
        });
        *last_visible = Some(projected_id);
    }
}
