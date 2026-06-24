use super::{
    policy::{Principal, PrincipalKind, ScopePath, Visibility},
    projection::{Projection, project_graph},
    repo_actions::preview_publish_import,
    store::{RepoPublicationState, StoredRepository, pending_import_scope_path},
};
use crate::error::ApiError;
use sha1::{Digest, Sha1};
use std::collections::{BTreeMap, HashSet};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ProjectionAudience {
    Owner,
    Public,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ProjectionSource {
    Live,
    Review,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ProjectionPreviewView {
    pub(crate) repo_id: String,
    pub(crate) principal_id: String,
    pub(crate) head_oid: Option<String>,
    pub(crate) files: Vec<ProjectionPreviewFile>,
    pub(crate) commits: Vec<ProjectionPreviewCommit>,
    pub(crate) summary: ProjectionPreviewSummary,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ProjectionPreviewFile {
    pub(crate) path: ScopePath,
    pub(crate) oid: String,
    pub(crate) visibility: Visibility,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ProjectionPreviewCommit {
    pub(crate) projected_id: String,
    pub(crate) logical_commit_id: String,
    pub(crate) parent_projected_id: Option<String>,
    pub(crate) author: Option<String>,
    pub(crate) message: String,
    pub(crate) synthetic: bool,
    pub(crate) visibility: ProjectionPreviewCommitVisibility,
    pub(crate) change_count: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ProjectionPreviewCommitVisibility {
    FullyPublic,
    Synthetic,
    Mixed,
    FullyPrivate,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ProjectionPreviewSummary {
    pub(crate) visible_files: usize,
    pub(crate) hidden_files: usize,
    pub(crate) visible_commits: usize,
    pub(crate) hidden_commits: usize,
    pub(crate) synthetic_commits: usize,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ProjectionViewFile {
    pub(crate) path: ScopePath,
    pub(crate) oid: String,
    pub(crate) tracked: bool,
    pub(crate) visibility: Visibility,
}

pub(crate) fn repo_for_projection_preview(
    repo: &StoredRepository,
    source: ProjectionSource,
) -> Result<StoredRepository, ApiError> {
    let mut preview = repo.clone();
    match source {
        ProjectionSource::Live => Ok(preview),
        ProjectionSource::Review => {
            if preview.record.publication_state == RepoPublicationState::PendingPublish {
                preview_publish_import(&mut preview)?;
            } else if let Some(staged_update) = preview.staged_update.clone() {
                crate::git::import::apply_receive_pack_update(&mut preview, staged_update)?;
            } else {
                return Err(ApiError::bad_request("repo has no pending review"));
            }
            Ok(preview)
        }
    }
}

pub(crate) fn projection_preview(
    repo: &StoredRepository,
    audience: ProjectionAudience,
    include_private_counts: bool,
) -> ProjectionPreviewView {
    let principal = projection_preview_principal(repo, audience);
    let projection = project_graph(&repo.policy, &repo.graph, &principal);
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
            synthetic: commit.synthetic,
            visibility: if commit.synthetic {
                ProjectionPreviewCommitVisibility::Synthetic
            } else {
                logical_commit_visibility
                    .get(commit.logical_commit_id.as_str())
                    .copied()
                    .unwrap_or(ProjectionPreviewCommitVisibility::FullyPublic)
            },
            change_count: commit.changes.len(),
        })
        .collect::<Vec<_>>();
    let synthetic_commits = commits.iter().filter(|commit| commit.synthetic).count();
    let visible_files = files.len();
    let visible_commits = commits.len();
    let (hidden_files, hidden_commits) =
        if audience == ProjectionAudience::Public && include_private_counts {
            let owner_projection = project_graph(
                &repo.policy,
                &repo.graph,
                &Principal {
                    id: repo.record.owner_user_id.clone(),
                    kind: PrincipalKind::User,
                },
            );
            let owner_files = projection_preview_files(repo, &owner_projection);
            (
                owner_files.len().saturating_sub(visible_files),
                hidden_logical_commit_count(&owner_projection, &projection),
            )
        } else {
            (0, 0)
        };

    ProjectionPreviewView {
        repo_id: repo.record.id.clone(),
        principal_id: projection.principal_id,
        head_oid,
        files,
        commits,
        summary: ProjectionPreviewSummary {
            visible_files,
            hidden_files,
            visible_commits,
            hidden_commits,
            synthetic_commits,
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

pub(crate) fn projected_files(
    repo: &StoredRepository,
    principal: &Principal,
) -> Vec<ProjectionViewFile> {
    let projection = project_graph(&repo.policy, &repo.graph, principal);

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

pub(crate) fn pending_import_files(
    repo: &StoredRepository,
    principal: &Principal,
) -> Result<Vec<ProjectionViewFile>, ApiError> {
    let Some(pending) = repo.pending_import.as_ref() else {
        return Ok(Vec::new());
    };
    let mut files = Vec::new();
    for file in &pending.files {
        let path = pending_scope_path(&file.path)?;
        if !repo.policy.can_read(principal, &path) {
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

pub(crate) fn files_for_visibility_update(
    repo: &StoredRepository,
    principal: &Principal,
) -> Result<Vec<ProjectionViewFile>, ApiError> {
    if repo.record.publication_state == RepoPublicationState::PendingPublish {
        pending_import_files(repo, principal)
    } else {
        Ok(projected_files(repo, principal))
    }
}

pub(crate) fn pending_scope_path(path: &str) -> Result<ScopePath, ApiError> {
    pending_import_scope_path(path).map_err(ApiError::bad_request)
}

pub(crate) fn has_visible_projected_files(repo: &StoredRepository, principal: &Principal) -> bool {
    let projection = project_graph(&repo.policy, &repo.graph, principal);
    !projection_tree(&projection).is_empty()
}

fn projection_preview_principal(
    repo: &StoredRepository,
    audience: ProjectionAudience,
) -> Principal {
    match audience {
        ProjectionAudience::Owner => Principal {
            id: repo.record.owner_user_id.clone(),
            kind: PrincipalKind::User,
        },
        ProjectionAudience::Public => Principal::public(),
    }
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
            projection.principal_id, commit.projected_id, tree_payload
        );
        hasher.update(format!("commit {}\0", payload.len()).as_bytes());
        hasher.update(payload.as_bytes());
        hex::encode(hasher.finalize())
    })
}
