use super::{
    policy::{Principal, ScopePath, Visibility},
    projection::{Projection, ProjectionViewKey, project_graph},
    store::pending_import_scope_path,
    store::{SourceBlob, StoredRepository},
};
use crate::error::ApiError;
use sha1::{Digest, Sha1};
use std::collections::{BTreeMap, HashSet};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProjectionAudience {
    Private,
    Public,
}

impl From<ProjectionAudience> for ProjectionViewKey {
    fn from(audience: ProjectionAudience) -> Self {
        match audience {
            ProjectionAudience::Private => Self::Private,
            ProjectionAudience::Public => Self::Public,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProjectionSource {
    Live,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProjectionPreviewView {
    pub repo_id: String,
    pub view_key: String,
    pub head_oid: Option<String>,
    pub files: Vec<ProjectionPreviewFile>,
    pub commits: Vec<ProjectionPreviewCommit>,
    pub summary: ProjectionPreviewSummary,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProjectionPreviewFile {
    pub path: ScopePath,
    pub oid: String,
    pub visibility: Visibility,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProjectionPreviewCommit {
    pub projected_id: String,
    pub logical_commit_id: String,
    pub parent_projected_id: Option<String>,
    pub author: Option<String>,
    pub message: String,
    pub visibility: ProjectionPreviewCommitVisibility,
    pub change_count: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProjectionPreviewCommitVisibility {
    FullyPublic,
    Mixed,
    FullyPrivate,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProjectionPreviewSummary {
    pub visible_files: usize,
    pub hidden_files: usize,
    pub visible_commits: usize,
    pub hidden_commits: usize,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProjectionViewFile {
    pub path: ScopePath,
    pub oid: String,
    pub tracked: bool,
    pub visibility: Visibility,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProjectionViewFileContent {
    pub file: ProjectionViewFile,
    pub blob: SourceBlob,
}

pub fn repo_for_projection_preview(
    repo: &StoredRepository,
    source: ProjectionSource,
) -> Result<StoredRepository, ApiError> {
    match source {
        ProjectionSource::Live => Ok(repo.clone()),
    }
}

pub fn projection_preview(
    repo: &StoredRepository,
    audience: ProjectionAudience,
    include_private_counts: bool,
) -> ProjectionPreviewView {
    let view_key = ProjectionViewKey::from(audience);
    let projection = project_graph(&repo.policy, &repo.graph, &repo.visibility_events, view_key);
    let files = projection_preview_files(repo, &projection);
    let head_oid = projection_preview_head_oid(&projection, &files);
    let logical_commit_visibility = repo
        .graph
        .commits
        .iter()
        .map(|commit| {
            (
                commit.id.as_str(),
                projection_preview_commit_visibility(commit),
            )
        })
        .collect::<BTreeMap<_, _>>();
    let commits = projection
        .commits
        .iter()
        .map(|commit| ProjectionPreviewCommit {
            projected_id: commit.projected_id.clone(),
            logical_commit_id: commit.logical_commit_id.clone(),
            parent_projected_id: commit.parent_projected_id.clone(),
            author: commit.author.clone(),
            message: commit.message.clone(),
            visibility: logical_commit_visibility
                .get(commit.logical_commit_id.as_str())
                .copied()
                .unwrap_or(ProjectionPreviewCommitVisibility::FullyPublic),
            change_count: commit.changes.len(),
        })
        .collect::<Vec<_>>();
    let visible_files = files.len();
    let visible_commits = commits.len();
    let (hidden_files, hidden_commits) =
        if audience == ProjectionAudience::Public && include_private_counts {
            let private_projection = project_graph(
                &repo.policy,
                &repo.graph,
                &repo.visibility_events,
                ProjectionViewKey::Private,
            );
            let private_files = projection_preview_files(repo, &private_projection);
            (
                private_files.len().saturating_sub(visible_files),
                hidden_logical_commit_count(&private_projection, &projection),
            )
        } else {
            (0, 0)
        };

    ProjectionPreviewView {
        repo_id: repo.record.id.clone(),
        view_key: projection.view_key.as_str().to_string(),
        head_oid,
        files,
        commits,
        summary: ProjectionPreviewSummary {
            visible_files,
            hidden_files,
            visible_commits,
            hidden_commits,
        },
    }
}

fn projection_preview_commit_visibility(
    commit: &super::projection::LogicalCommit,
) -> ProjectionPreviewCommitVisibility {
    if !commit.changes.is_empty()
        && commit
            .changes
            .iter()
            .all(|change| change.visibility == Visibility::Private)
    {
        return ProjectionPreviewCommitVisibility::FullyPrivate;
    }

    if commit
        .changes
        .iter()
        .all(|change| change.visibility == Visibility::Public)
    {
        return ProjectionPreviewCommitVisibility::FullyPublic;
    }

    ProjectionPreviewCommitVisibility::Mixed
}

pub fn projected_files(repo: &StoredRepository, principal: &Principal) -> Vec<ProjectionViewFile> {
    let access = repo.access_for_principal(principal);
    let projection = project_graph(
        &repo.policy,
        &repo.graph,
        &repo.visibility_events,
        ProjectionViewKey::from_access(access),
    );

    projection_tree(&projection)
        .into_iter()
        .map(|(path, oid)| ProjectionViewFile {
            visibility: repo.policy.effective_visibility(&path),
            path,
            oid,
            tracked: true,
        })
        .collect()
}

pub fn projected_file_content(
    repo: &StoredRepository,
    principal: &Principal,
    path: &ScopePath,
) -> Option<ProjectionViewFileContent> {
    let access = repo.access_for_principal(principal);
    let projection = project_graph(
        &repo.policy,
        &repo.graph,
        &repo.visibility_events,
        ProjectionViewKey::from_access(access),
    );
    let blob = projection
        .commits
        .iter()
        .rev()
        .flat_map(|commit| commit.changes.iter().rev())
        .find(|change| &change.path == path)?
        .new_content
        .clone()?;
    Some(ProjectionViewFileContent {
        file: ProjectionViewFile {
            path: path.clone(),
            oid: blob.git_oid.clone(),
            tracked: true,
            visibility: repo.policy.effective_visibility(path),
        },
        blob,
    })
}

pub fn pending_import_files(
    repo: &StoredRepository,
    principal: &Principal,
) -> Result<Vec<ProjectionViewFile>, ApiError> {
    let Some(pending) = repo.pending_import.as_ref() else {
        return Ok(Vec::new());
    };
    let mut files = Vec::new();
    for file in &pending.files {
        let path = pending_scope_path(&file.path)?;
        let access = repo.access_for_principal(principal);
        if !repo.policy.can_read(&path, access.can_read_private_files) {
            continue;
        }
        files.push(ProjectionViewFile {
            oid: file.oid.clone(),
            tracked: true,
            visibility: repo.policy.effective_visibility(&path),
            path,
        });
    }
    files.sort_by(|left, right| left.path.cmp(&right.path));
    Ok(files)
}

pub fn files_for_visibility_update(
    repo: &StoredRepository,
    principal: &Principal,
) -> Result<Vec<ProjectionViewFile>, ApiError> {
    if repo.has_pending_import_review() {
        pending_import_files(repo, principal)
    } else {
        Ok(projected_files(repo, principal))
    }
}

pub fn pending_scope_path(path: &str) -> Result<ScopePath, ApiError> {
    pending_import_scope_path(path).map_err(ApiError::bad_request)
}

pub fn has_visible_projected_files(repo: &StoredRepository, principal: &Principal) -> bool {
    let projection = project_graph(
        &repo.policy,
        &repo.graph,
        &repo.visibility_events,
        ProjectionViewKey::from_access(repo.access_for_principal(principal)),
    );
    !projection_tree(&projection).is_empty()
}

pub fn has_visible_projected_history(repo: &StoredRepository, principal: &Principal) -> bool {
    let projection = project_graph(
        &repo.policy,
        &repo.graph,
        &repo.visibility_events,
        ProjectionViewKey::from_access(repo.access_for_principal(principal)),
    );
    !projection.commits.is_empty()
}

fn projection_preview_files(
    repo: &StoredRepository,
    projection: &Projection,
) -> Vec<ProjectionPreviewFile> {
    projection_tree(projection)
        .into_iter()
        .map(|(path, oid)| ProjectionPreviewFile {
            visibility: repo.policy.effective_visibility(&path),
            path,
            oid,
        })
        .collect()
}

fn hidden_logical_commit_count(owner_projection: &Projection, projection: &Projection) -> usize {
    let visible_logical_ids = projection
        .commits
        .iter()
        .map(|commit| commit.logical_commit_id.as_str())
        .collect::<HashSet<_>>();

    owner_projection
        .commits
        .iter()
        .filter(|commit| !visible_logical_ids.contains(commit.logical_commit_id.as_str()))
        .count()
}

fn projection_tree(projection: &Projection) -> BTreeMap<ScopePath, String> {
    let mut tree = BTreeMap::new();
    for change in projection
        .commits
        .iter()
        .flat_map(|commit| commit.changes.iter())
    {
        match &change.new_content {
            Some(blob) => {
                tree.insert(change.path.clone(), blob.git_oid.clone());
            }
            None => {
                tree.remove(&change.path);
            }
        }
    }
    tree
}

fn projection_preview_head_oid(
    projection: &Projection,
    files: &[ProjectionPreviewFile],
) -> Option<String> {
    projection.commits.last().map(|commit| {
        let mut hasher = Sha1::new();
        let tree_payload = files
            .iter()
            .map(|file| format!("100644 blob {}\t{}", file.oid, file.path.as_str()))
            .collect::<Vec<_>>()
            .join("\n");
        let payload = format!(
            "projection:{}\nhead:{}\ntree:\n{}\n",
            projection.view_key.as_str(),
            commit.projected_id,
            tree_payload
        );
        hasher.update(format!("commit {}\0", payload.len()).as_bytes());
        hasher.update(payload.as_bytes());
        hex::encode(hasher.finalize())
    })
}
