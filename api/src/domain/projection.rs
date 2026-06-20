use super::{
    policy::{Policy, Principal, ScopePath},
    store::SourceBlob,
};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum MixedCommitPolicy {
    SyntheticPublicCommit,
    OmitFromPublic,
}

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
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LogicalCommit {
    pub id: String,
    pub parent_ids: Vec<String>,
    pub author_id: String,
    pub author_visibility: AuthorVisibility,
    pub message: String,
    pub mixed_policy: MixedCommitPolicy,
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
    pub synthetic: bool,
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

pub fn project_graph(policy: &Policy, graph: &SourceGraph, principal: &Principal) -> Projection {
    let mut commits = Vec::new();
    let mut last_visible: Option<String> = None;

    for logical in &graph.commits {
        let visible_changes = logical
            .changes
            .iter()
            .filter(|change| policy.can_read(principal, &change.path))
            .map(|change| ProjectedChange {
                path: change.path.clone(),
                new_content: change.new_content.clone(),
            })
            .collect::<Vec<_>>();

        if visible_changes.is_empty() {
            continue;
        }

        let hidden_count = logical.changes.len() - visible_changes.len();
        if hidden_count > 0 && logical.mixed_policy == MixedCommitPolicy::OmitFromPublic {
            continue;
        }

        let synthetic = hidden_count > 0;
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
            author: match logical.author_visibility {
                AuthorVisibility::Visible => Some(logical.author_id.clone()),
                AuthorVisibility::Hidden => None,
            },
            message: if synthetic {
                "Synthetic public projection".to_string()
            } else {
                logical.message.clone()
            },
            synthetic,
            changes: visible_changes,
        });

        last_visible = Some(projected_id);
    }

    Projection {
        repo_id: graph.repo_id.clone(),
        principal_id: principal.id.clone(),
        commits,
    }
}
