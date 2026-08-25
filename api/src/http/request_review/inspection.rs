use super::*;
use std::collections::{BTreeMap, BTreeSet};

const MAX_REQUEST_COMMIT_IDENTITY_BYTES: usize = 64 * 1024;
const MAX_REQUEST_COMMIT_METADATA_BYTES: usize = 64 * 1024;

pub(super) fn request_revision_commit_files(
    raw_repo: &FsPath,
    repo: &StoredRepository,
    access: RepositoryAccess,
    revision: &RequestRevision,
    commit_oid: &str,
) -> Result<InspectedRequestCommitFiles, ApiError> {
    if !commit_belongs_to_revision(raw_repo, revision, commit_oid)? {
        return Err(ApiError::not_found("request revision commit not found"));
    }
    let inspected = inspect_request_commit(raw_repo, repo, access, commit_oid)?;
    let commit = inspected
        .commit
        .ok_or_else(|| ApiError::not_found("request revision commit not found"))?;
    Ok(InspectedRequestCommitFiles { commit })
}

pub(super) struct InspectedRequestCommitFiles {
    pub(super) commit: RequestRevisionCommitResponse,
}

fn request_commit_is_visible_to(
    raw_repo: &FsPath,
    repo: &StoredRepository,
    access: RepositoryAccess,
    commit_oid: &str,
) -> Result<bool, ApiError> {
    let identity = request_commit_identity(raw_repo, commit_oid)?;
    request_commit_changes(raw_repo, repo, access, &identity.parent_oids, commit_oid)
        .map(|changes| !changes.hidden)
}

pub(crate) struct RequestRevisionCommitVisibility<'a> {
    state: &'a AppState,
    owner: &'a str,
    repo_name: &'a str,
    repo: &'a StoredRepository,
    access: RepositoryAccess,
    request: &'a Request,
}

impl<'a> RequestRevisionCommitVisibility<'a> {
    pub(crate) fn new(
        state: &'a AppState,
        owner: &'a str,
        repo_name: &'a str,
        repo: &'a StoredRepository,
        access: RepositoryAccess,
        request: &'a Request,
    ) -> Self {
        Self {
            state,
            owner,
            repo_name,
            repo,
            access,
            request,
        }
    }

    pub(crate) async fn visible_commits(
        &self,
        commits_by_revision: &BTreeMap<String, BTreeSet<String>>,
    ) -> BTreeSet<(String, String)> {
        let mut visible = BTreeSet::new();
        for (revision_id, commit_oids) in commits_by_revision {
            let result = self
                .visible_commits_in_revision(revision_id, commit_oids)
                .await;
            match result {
                Ok(commit_oids) => visible.extend(
                    commit_oids
                        .into_iter()
                        .map(|commit_oid| (revision_id.clone(), commit_oid)),
                ),
                Err(error) => tracing::warn!(
                    request_id = %self.request.id,
                    revision_id,
                    error = ?error,
                    "redacting discussion anchors because request revision inspection failed"
                ),
            }
        }
        visible
    }

    async fn visible_commits_in_revision(
        &self,
        revision_id: &str,
        commit_oids: &BTreeSet<String>,
    ) -> Result<BTreeSet<String>, ApiError> {
        let Some(revision) = self
            .state
            .metadata
            .requests()
            .request_revision(&self.request.id, revision_id)
            .await?
        else {
            return Ok(BTreeSet::new());
        };
        with_request_revision_store_repo(
            self.state,
            self.owner,
            self.repo_name,
            self.request,
            &revision,
            |raw_repo| {
                let mut visible = BTreeSet::new();
                for commit_oid in commit_oids {
                    if commit_belongs_to_revision(raw_repo, &revision, commit_oid)?
                        && request_commit_is_visible_to(
                            raw_repo,
                            self.repo,
                            self.access,
                            commit_oid,
                        )?
                    {
                        visible.insert(commit_oid.clone());
                    }
                }
                Ok(visible)
            },
        )
    }
}

pub(super) fn inspect_request_commit(
    raw_repo: &FsPath,
    repo: &StoredRepository,
    access: RepositoryAccess,
    commit_oid: &str,
) -> Result<InspectedRequestCommit, ApiError> {
    let identity = request_commit_identity(raw_repo, commit_oid)?;
    let changes =
        request_commit_changes(raw_repo, repo, access, &identity.parent_oids, commit_oid)?;
    if changes.hidden {
        return Ok(InspectedRequestCommit {
            commit: None,
            inspection: RequestRevisionInspectionState::Complete,
        });
    }
    let metadata = request_commit_display_metadata(raw_repo, commit_oid)?;
    Ok(InspectedRequestCommit {
        commit: Some(RequestRevisionCommitResponse {
            oid: commit_oid.to_string(),
            parent_oids: if access.can_read_private_files {
                identity.parent_oids
            } else {
                Vec::new()
            },
            author: metadata.author,
            authored_at_unix: identity.authored_at_unix,
            message: metadata.message,
            change_count: changes.files.len(),
            files: changes.files,
        }),
        inspection: if metadata.complete {
            RequestRevisionInspectionState::Complete
        } else {
            RequestRevisionInspectionState::Incomplete
        },
    })
}

pub(super) struct InspectedRequestCommit {
    pub(super) commit: Option<RequestRevisionCommitResponse>,
    pub(super) inspection: RequestRevisionInspectionState,
}

fn request_commit_identity(
    raw_repo: &FsPath,
    commit_oid: &str,
) -> Result<RequestCommitIdentity, ApiError> {
    let output = run_git_output_bounded(
        Some(raw_repo),
        &["show", "-s", "--format=%P%x00%at", commit_oid],
        "reading request commit identity",
        MAX_REQUEST_COMMIT_IDENTITY_BYTES,
    )?;
    if !output.status.success() {
        return Err(ApiError::infrastructure_unavailable(format!(
            "reading request commit identity: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        )));
    }
    let identity = parse_request_commit_identity(&output.stdout)?;
    if identity.parent_oids.is_empty() {
        return Err(ApiError::conflict(
            "request revision commit must have a parent",
        ));
    }
    Ok(identity)
}

fn request_commit_changes(
    raw_repo: &FsPath,
    repo: &StoredRepository,
    access: RepositoryAccess,
    parent_oids: &[String],
    commit_oid: &str,
) -> Result<VisibleRequestChanges, ApiError> {
    // Request-ref validation requires every revision head to descend from its recorded base,
    // so commits introduced by a revision cannot be parentless roots.
    let parent = parent_oids
        .first()
        .ok_or_else(|| ApiError::conflict("request revision commit must have a parent"))?;
    request_changes_from_repo_with_visibility(raw_repo, repo, access, parent, commit_oid, None)
}

fn request_commit_display_metadata(
    raw_repo: &FsPath,
    commit_oid: &str,
) -> Result<RequestCommitDisplayMetadata, ApiError> {
    let output = match run_git_output_bounded(
        Some(raw_repo),
        &["show", "-s", "--format=%an <%ae>%x00%B", commit_oid],
        "reading request commit metadata",
        MAX_REQUEST_COMMIT_METADATA_BYTES,
    ) {
        Ok(output) => output,
        Err(error) if error.status() == axum::http::StatusCode::PAYLOAD_TOO_LARGE => {
            return Ok(RequestCommitDisplayMetadata {
                author: None,
                message: String::new(),
                complete: false,
            });
        }
        Err(error) => return Err(error),
    };
    if !output.status.success() {
        return Err(ApiError::infrastructure_unavailable(format!(
            "reading request commit metadata: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        )));
    }
    parse_request_commit_display_metadata(&output.stdout)
}

struct RequestCommitIdentity {
    parent_oids: Vec<String>,
    authored_at_unix: u64,
}

struct RequestCommitDisplayMetadata {
    author: Option<String>,
    message: String,
    complete: bool,
}

fn parse_request_commit_identity(output: &[u8]) -> Result<RequestCommitIdentity, ApiError> {
    let mut fields = output.splitn(2, |byte| *byte == 0);
    let parent_oids = fields
        .next()
        .unwrap_or_default()
        .split(|byte| byte.is_ascii_whitespace())
        .filter(|value| !value.is_empty())
        .map(|value| String::from_utf8_lossy(value).into_owned())
        .collect::<Vec<_>>();
    let authored_at_unix = std::str::from_utf8(fields.next().unwrap_or_default())
        .map_err(ApiError::bad_request)?
        .trim()
        .parse::<u64>()
        .map_err(ApiError::bad_request)?;
    Ok(RequestCommitIdentity {
        parent_oids,
        authored_at_unix,
    })
}

fn parse_request_commit_display_metadata(
    output: &[u8],
) -> Result<RequestCommitDisplayMetadata, ApiError> {
    let mut fields = output.splitn(2, |byte| *byte == 0);
    let author = fields
        .next()
        .map(String::from_utf8_lossy)
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty());
    let message = String::from_utf8_lossy(fields.next().unwrap_or_default())
        .trim()
        .to_string();
    Ok(RequestCommitDisplayMetadata {
        author,
        message,
        complete: true,
    })
}

#[cfg(test)]
mod tests {
    use super::{parse_request_commit_display_metadata, parse_request_commit_identity};

    #[test]
    fn commit_metadata_decodes_non_utf8_display_fields_lossily() {
        let identity = parse_request_commit_identity(
            b"0123456789012345678901234567890123456789\0 1700000000\n",
        )
        .unwrap();
        let metadata = parse_request_commit_display_metadata(
            b"Ada \xff <ada@example.com>\0Fix \xfe metadata\n",
        )
        .unwrap();

        assert_eq!(identity.parent_oids.len(), 1);
        assert_eq!(identity.authored_at_unix, 1_700_000_000);
        assert_eq!(metadata.author.as_deref(), Some("Ada � <ada@example.com>"));
        assert_eq!(metadata.message, "Fix � metadata");
        assert!(metadata.complete);
    }
}
