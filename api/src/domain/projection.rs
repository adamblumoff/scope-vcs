use super::{
    policy::{Policy, Principal, PrincipalKind, ScopePath, Visibility},
    store::SourceBlob,
};
use serde::{Deserialize, Serialize};

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

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Projection {
    pub repo_id: String,
    pub principal_id: String,
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
    principal: &Principal,
) -> Projection {
    let mut commits = Vec::new();
    let mut last_visible: Option<String> = None;
    let owner_projection = policy.is_owner(principal);
    let commit_positions = graph
        .commits
        .iter()
        .enumerate()
        .map(|(index, commit)| (commit.id.as_str(), index))
        .collect::<std::collections::BTreeMap<_, _>>();
    let readable_epoch_starts =
        readable_epoch_starts(policy, principal, visibility_events, &commit_positions);
    let baseline_events = if owner_projection {
        Vec::new()
    } else {
        baseline_events(policy, principal, visibility_events, &readable_epoch_starts)
    };

    process_baseline_events_after(
        &mut commits,
        &mut last_visible,
        &baseline_events,
        None,
        principal,
    );

    for (commit_index, logical) in graph.commits.iter().enumerate() {
        let visible_changes = logical
            .changes
            .iter()
            .filter(|change| {
                owner_projection
                    || can_read_historical_change(
                        policy,
                        principal,
                        &change.path,
                        change.visibility,
                    ) && is_change_in_readable_epoch(
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
                principal,
            );
            continue;
        }

        let partial = visible_content_count < logical.changes.len();
        let projected_id = format!(
            "pv_{}_{}_{}",
            principal.id.replace(['/', ':'], "_"),
            logical.id,
            commits.len() + 1
        );

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
            principal,
        );
    }

    Projection {
        repo_id: graph.repo_id.clone(),
        principal_id: principal.id.clone(),
        commits,
    }
}

fn can_read_historical_change(
    policy: &Policy,
    principal: &Principal,
    path: &ScopePath,
    visibility: Visibility,
) -> bool {
    match visibility {
        Visibility::Public => policy.can_read(principal, path),
        Visibility::Private => {
            principal.kind != PrincipalKind::Public
                && policy.effective_visibility(path) == Visibility::Private
                && policy.can_read(principal, path)
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct ReadableEpochStart {
    source_commit_index: Option<usize>,
    baseline_event_id: Option<String>,
}

fn readable_epoch_starts(
    policy: &Policy,
    principal: &Principal,
    events: &[VisibilityEvent],
    commit_positions: &std::collections::BTreeMap<&str, usize>,
) -> std::collections::BTreeMap<ScopePath, ReadableEpochStart> {
    let mut starts = std::collections::BTreeMap::new();
    for event in events {
        if !policy.can_read(principal, &event.path) {
            continue;
        }

        let old_readable =
            visibility_readable_for_event(policy, principal, &event.path, event.old_visibility);
        let new_readable =
            visibility_readable_for_event(policy, principal, &event.path, event.new_visibility);
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
    principal: &Principal,
    path: &ScopePath,
    visibility: Visibility,
) -> bool {
    match visibility {
        Visibility::Public => true,
        Visibility::Private => {
            principal.kind != PrincipalKind::Public
                && policy.effective_visibility(path) == Visibility::Private
                && policy.can_read(principal, path)
        }
    }
}

fn is_change_in_readable_epoch(
    starts: &std::collections::BTreeMap<ScopePath, ReadableEpochStart>,
    path: &ScopePath,
    commit_index: usize,
) -> bool {
    starts
        .get(path)
        .and_then(|start| start.source_commit_index)
        .is_none_or(|start_index| commit_index >= start_index)
}

fn baseline_events<'a>(
    policy: &Policy,
    principal: &Principal,
    events: &'a [VisibilityEvent],
    starts: &std::collections::BTreeMap<ScopePath, ReadableEpochStart>,
) -> Vec<&'a VisibilityEvent> {
    events
        .iter()
        .filter(|event| {
            starts
                .get(&event.path)
                .and_then(|start| start.baseline_event_id.as_deref())
                == Some(event.id.as_str())
                && policy.can_read(principal, &event.path)
                && event.current_content.is_some()
        })
        .collect()
}

fn process_baseline_events_after(
    commits: &mut Vec<ProjectedCommit>,
    last_visible: &mut Option<String>,
    baseline_events: &[&VisibilityEvent],
    after_commit_id: Option<&str>,
    principal: &Principal,
) {
    for event in baseline_events
        .iter()
        .filter(|event| event.after_commit_id.as_deref() == after_commit_id)
    {
        let projected_id = format!(
            "pv_{}_{}_{}",
            principal.id.replace(['/', ':'], "_"),
            event.id,
            commits.len() + 1
        );
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
