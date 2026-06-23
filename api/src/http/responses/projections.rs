use crate::domain::policy::{Principal, PrincipalKind, ScopePath, Visibility};
use crate::domain::projection::{Projection, project_graph};
use crate::domain::store::{RepoPublicationState, StoredRepository, pending_import_scope_path};
use crate::{
    error::ApiError,
    object_store::{ObjectStore, source_blob_text},
};
use serde::{Deserialize, Serialize};
use sha1::{Digest, Sha1};
use std::collections::{BTreeMap, HashSet};

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum ProjectionPreviewAudience {
    Owner,
    Public,
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum ProjectionPreviewSource {
    Live,
    Review,
}

#[derive(Debug, Deserialize)]
pub(crate) struct ProjectionPreviewRequest {
    pub(crate) audience: ProjectionPreviewAudience,
    pub(crate) source: Option<ProjectionPreviewSource>,
}

#[derive(Debug, Serialize)]
pub(crate) struct ProjectionPreviewResponse {
    pub(crate) audience: ProjectionPreviewAudience,
    pub(crate) source: ProjectionPreviewSource,
    pub(crate) repo_id: String,
    pub(crate) principal_id: String,
    pub(crate) head_oid: Option<String>,
    pub(crate) files: Vec<ProjectionPreviewFileResponse>,
    pub(crate) commits: Vec<ProjectionPreviewCommitResponse>,
    pub(crate) summary: ProjectionPreviewSummaryResponse,
}

#[derive(Debug, Serialize)]
pub(crate) struct ProjectionPreviewFileResponse {
    pub(crate) path: String,
    pub(crate) oid: String,
    pub(crate) visibility: Visibility,
}

#[derive(Debug, Serialize)]
pub(crate) struct ProjectionPreviewCommitResponse {
    pub(crate) projected_id: String,
    pub(crate) logical_commit_id: String,
    pub(crate) parent_projected_id: Option<String>,
    pub(crate) author: Option<String>,
    pub(crate) message: String,
    pub(crate) synthetic: bool,
    pub(crate) change_count: usize,
}

#[derive(Debug, Serialize)]
pub(crate) struct ProjectionPreviewSummaryResponse {
    pub(crate) visible_files: usize,
    pub(crate) hidden_files: usize,
    pub(crate) visible_commits: usize,
    pub(crate) hidden_commits: usize,
    pub(crate) synthetic_commits: usize,
}

#[derive(Debug, Serialize)]
pub(crate) struct ProjectionResponse {
    pub(crate) repo_id: String,
    pub(crate) principal_id: String,
    pub(crate) commits: Vec<ProjectedCommitResponse>,
}

#[derive(Debug, Serialize)]
pub(crate) struct ProjectedCommitResponse {
    pub(crate) projected_id: String,
    pub(crate) logical_commit_id: String,
    pub(crate) parent_projected_id: Option<String>,
    pub(crate) author: Option<String>,
    pub(crate) message: String,
    pub(crate) synthetic: bool,
    pub(crate) changes: Vec<ProjectedChangeResponse>,
}

#[derive(Debug, Serialize)]
pub(crate) struct ProjectedChangeResponse {
    pub(crate) path: ScopePath,
    pub(crate) new_content: Option<String>,
}

#[derive(Debug, Serialize)]
pub(crate) struct RepoFileResponse {
    pub(crate) path: String,
    pub(crate) oid: String,
    pub(crate) tracked: bool,
    pub(crate) visibility: Visibility,
}

pub(crate) fn projection_response(
    store: &dyn ObjectStore,
    projection: Projection,
) -> Result<ProjectionResponse, ApiError> {
    Ok(ProjectionResponse {
        repo_id: projection.repo_id,
        principal_id: projection.principal_id,
        commits: projection
            .commits
            .into_iter()
            .map(|commit| {
                let changes = commit
                    .changes
                    .into_iter()
                    .map(|change| {
                        Ok(ProjectedChangeResponse {
                            path: change.path,
                            new_content: change
                                .new_content
                                .as_ref()
                                .map(|blob| source_blob_text(store, blob))
                                .transpose()?,
                        })
                    })
                    .collect::<Result<Vec<_>, ApiError>>()?;
                Ok(ProjectedCommitResponse {
                    projected_id: commit.projected_id,
                    logical_commit_id: commit.logical_commit_id,
                    parent_projected_id: commit.parent_projected_id,
                    author: commit.author,
                    message: commit.message,
                    synthetic: commit.synthetic,
                    changes,
                })
            })
            .collect::<Result<Vec<_>, ApiError>>()?,
    })
}

pub(crate) fn projection_preview_response(
    repo: &StoredRepository,
    audience: ProjectionPreviewAudience,
    source: ProjectionPreviewSource,
    include_private_counts: bool,
) -> Result<ProjectionPreviewResponse, ApiError> {
    let principal = projection_preview_principal(repo, audience);
    let projection = project_graph(&repo.policy, &repo.graph, &principal);
    let files = projection_preview_files(repo, &projection);
    let head_oid = projection_preview_head_oid(&projection, &files);
    let commits = projection
        .commits
        .iter()
        .map(|commit| ProjectionPreviewCommitResponse {
            projected_id: commit.projected_id.clone(),
            logical_commit_id: commit.logical_commit_id.clone(),
            parent_projected_id: commit.parent_projected_id.clone(),
            author: commit.author.clone(),
            message: commit.message.clone(),
            synthetic: commit.synthetic,
            change_count: commit.changes.len(),
        })
        .collect::<Vec<_>>();
    let synthetic_commits = commits.iter().filter(|commit| commit.synthetic).count();
    let visible_files = files.len();
    let visible_commits = commits.len();
    let (hidden_files, hidden_commits) =
        if audience == ProjectionPreviewAudience::Public && include_private_counts {
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

    Ok(ProjectionPreviewResponse {
        audience,
        source,
        repo_id: repo.record.id.clone(),
        principal_id: projection.principal_id,
        head_oid,
        files,
        commits,
        summary: ProjectionPreviewSummaryResponse {
            visible_files,
            hidden_files,
            visible_commits,
            hidden_commits,
            synthetic_commits,
        },
    })
}

pub(crate) fn projected_files(
    repo: &StoredRepository,
    principal: &Principal,
) -> Result<Vec<RepoFileResponse>, ApiError> {
    let projection = project_graph(&repo.policy, &repo.graph, principal);

    Ok(projection_tree(&projection)
        .into_iter()
        .map(|(path, oid)| RepoFileResponse {
            visibility: repo.policy.effective_visibility(&path),
            path: path.as_str().to_string(),
            oid,
            tracked: true,
        })
        .collect())
}

pub(crate) fn pending_import_files(
    repo: &StoredRepository,
    principal: &Principal,
) -> Result<Vec<RepoFileResponse>, ApiError> {
    let Some(pending) = repo.pending_import.as_ref() else {
        return Ok(Vec::new());
    };
    let mut files = Vec::new();
    for file in &pending.files {
        let path = pending_scope_path(&file.path)?;
        if !repo.policy.can_read(principal, &path) {
            continue;
        }
        files.push(RepoFileResponse {
            path: path.as_str().to_string(),
            oid: file.oid.clone(),
            tracked: true,
            visibility: repo.policy.effective_visibility(&path),
        });
    }
    files.sort_by(|left, right| left.path.cmp(&right.path));
    Ok(files)
}

pub(crate) fn files_for_visibility_update(
    repo: &StoredRepository,
    principal: &Principal,
) -> Result<Vec<RepoFileResponse>, ApiError> {
    if repo.record.publication_state == RepoPublicationState::PendingPublish {
        pending_import_files(repo, principal)
    } else {
        projected_files(repo, principal)
    }
}

pub(crate) fn pending_scope_path(path: &str) -> Result<ScopePath, ApiError> {
    pending_import_scope_path(path).map_err(ApiError::bad_request)
}

fn projection_preview_principal(
    repo: &StoredRepository,
    audience: ProjectionPreviewAudience,
) -> Principal {
    match audience {
        ProjectionPreviewAudience::Owner => Principal {
            id: repo.record.owner_user_id.clone(),
            kind: PrincipalKind::User,
        },
        ProjectionPreviewAudience::Public => Principal::public(),
    }
}

fn projection_preview_files(
    repo: &StoredRepository,
    projection: &Projection,
) -> Vec<ProjectionPreviewFileResponse> {
    projection_tree(projection)
        .into_iter()
        .map(|(path, oid)| ProjectionPreviewFileResponse {
            visibility: repo.policy.effective_visibility(&path),
            path: path.as_str().to_string(),
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
    files: &[ProjectionPreviewFileResponse],
) -> Option<String> {
    projection.commits.last().map(|commit| {
        let mut hasher = Sha1::new();
        let tree_payload = files
            .iter()
            .map(|file| format!("100644 blob {}\t{}", file.oid, file.path))
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
