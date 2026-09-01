use super::*;
use crate::runtime_budgets::RuntimeBudgets;
use scope_git_process::{ProcessLimits, StreamingProcessError, run_with_stdout, truncated_stderr};
use std::{
    collections::{BTreeMap, BTreeSet},
    io::{BufRead, BufReader, Read},
    process::Command,
};

const MAX_REQUEST_COMMIT_IDENTITY_BYTES: usize = 64 * 1024;
const MAX_REQUEST_COMMIT_METADATA_BYTES: usize = 64 * 1024;
const MAX_REQUEST_DIFF_FIELD_BYTES: usize = 64 * 1024;

pub(super) fn request_revision_commit_files(
    raw_repo: &FsPath,
    repo: &Repository,
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
    repo: &Repository,
    access: RepositoryAccess,
    commit_oid: &str,
) -> Result<bool, ApiError> {
    let identity = request_commit_identity(raw_repo, commit_oid)?;
    request_commit_changes(raw_repo, repo, access, &identity.parent_oids, commit_oid)
        .map(|changes| !changes.hidden)
}

pub(crate) struct RequestRevisionCommitVisibility<'a> {
    state: &'a AppState,
    repo: &'a Repository,
    access: RepositoryAccess,
    request: &'a Request,
}

impl<'a> RequestRevisionCommitVisibility<'a> {
    pub(crate) fn new(
        state: &'a AppState,
        repo: &'a Repository,
        access: RepositoryAccess,
        request: &'a Request,
    ) -> Self {
        Self {
            state,
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
            &self.repo.incarnation(),
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
    repo: &Repository,
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
            files_truncated: false,
        }),
        inspection: if metadata.complete {
            RequestRevisionInspectionState::Complete
        } else {
            RequestRevisionInspectionState::Incomplete
        },
    })
}

pub(super) fn inspect_request_commits_identity_only(
    raw_repo: &FsPath,
    repo: &Repository,
    access: RepositoryAccess,
    commit_oids: &[String],
) -> Result<Vec<InspectedRequestCommit>, ApiError> {
    let mut changes = request_commit_change_summaries(raw_repo, repo, access, commit_oids)?;
    commit_oids
        .iter()
        .map(|commit_oid| {
            let changes = changes.remove(commit_oid).ok_or_else(|| {
                ApiError::internal_message("request identity diff omitted a commit")
            })?;
            if changes.hidden {
                return Ok(InspectedRequestCommit {
                    commit: None,
                    inspection: RequestRevisionInspectionState::Complete,
                });
            }
            let identity = request_commit_identity(raw_repo, commit_oid)?;
            let metadata = request_commit_display_metadata(raw_repo, commit_oid)?;
            Ok(InspectedRequestCommit {
                commit: Some(RequestRevisionCommitResponse {
                    oid: commit_oid.clone(),
                    parent_oids: if access.can_read_private_files {
                        identity.parent_oids
                    } else {
                        Vec::new()
                    },
                    author: metadata.author,
                    authored_at_unix: identity.authored_at_unix,
                    message: metadata.message,
                    change_count: changes.change_count,
                    files: Vec::new(),
                    files_truncated: changes.change_count != 0,
                }),
                inspection: if metadata.complete && changes.change_count == 0 {
                    RequestRevisionInspectionState::Complete
                } else {
                    RequestRevisionInspectionState::Incomplete
                },
            })
        })
        .collect()
}

pub(super) struct InspectedRequestCommit {
    pub(super) commit: Option<RequestRevisionCommitResponse>,
    pub(super) inspection: RequestRevisionInspectionState,
}

#[derive(Default)]
struct RequestCommitChangeSummary {
    change_count: usize,
    hidden: bool,
}

fn request_commit_change_summaries(
    raw_repo: &FsPath,
    repo: &Repository,
    access: RepositoryAccess,
    commit_oids: &[String],
) -> Result<BTreeMap<String, RequestCommitChangeSummary>, ApiError> {
    if commit_oids.is_empty() {
        return Ok(BTreeMap::new());
    }
    let expected = commit_oids.iter().cloned().collect::<BTreeSet<_>>();
    let policy = repo.policy.clone();
    let mut input = commit_oids.join("\n").into_bytes();
    input.push(b'\n');
    let mut command = Command::new("git");
    command.arg("-C").arg(raw_repo).args([
        "diff-tree",
        "--stdin",
        "--raw",
        "-r",
        "-z",
        "--no-renames",
        "--abbrev=64",
        "--diff-merges=first-parent",
        "--always",
    ]);
    let output = run_with_stdout(
        &mut command,
        Some(input),
        ProcessLimits::new(RuntimeBudgets::default_git_command_timeout()),
        "reading bounded request commit identities",
        move |stdout, _cancellation| {
            parse_request_commit_change_summaries(
                stdout,
                &policy,
                access.can_read_private_files,
                &expected,
            )
        },
    )
    .map_err(|error| match error {
        StreamingProcessError::Process(error) => {
            ApiError::infrastructure_unavailable(error.to_string())
        }
        StreamingProcessError::Consumer(error) => error,
    })?;
    if !output.status.success() {
        return Err(ApiError::infrastructure_unavailable(format!(
            "reading bounded request commit identities: {}",
            truncated_stderr(&output.stderr, scope_git_process::STDERR_DIAGNOSTIC_BYTES).trim()
        )));
    }
    Ok(output.value)
}

fn parse_request_commit_change_summaries(
    stdout: Box<dyn Read + Send>,
    policy: &scope_domain::policy::Policy,
    can_read_private_files: bool,
    expected: &BTreeSet<String>,
) -> Result<BTreeMap<String, RequestCommitChangeSummary>, ApiError> {
    let mut reader = BufReader::new(stdout);
    let mut summaries = BTreeMap::<String, RequestCommitChangeSummary>::new();
    let mut current_oid = None;
    while let Some(field) = read_nul_field_bounded(&mut reader)? {
        if field.bytes.starts_with(b":") {
            let oid = current_oid.as_ref().ok_or_else(|| {
                ApiError::internal_message("request identity diff is missing a commit header")
            })?;
            validate_request_diff_header(&field)?;
            let path = read_nul_field_bounded(&mut reader)?.ok_or_else(|| {
                ApiError::internal_message("request identity diff is missing a path")
            })?;
            let summary = summaries
                .get_mut(oid)
                .expect("current request identity diff commit must exist");
            summary.change_count = summary.change_count.checked_add(1).ok_or_else(|| {
                ApiError::internal_message("request identity diff change count overflowed")
            })?;
            if !can_read_private_files && !summary.hidden {
                summary.hidden = path.truncated || !request_path_is_public(policy, &path.bytes)?;
            }
            continue;
        }
        if field.truncated {
            return Err(ApiError::internal_message(
                "request identity diff commit header is too large",
            ));
        }
        let oid = std::str::from_utf8(&field.bytes)
            .map_err(ApiError::bad_request)?
            .to_string();
        if !expected.contains(&oid) || summaries.insert(oid.clone(), Default::default()).is_some() {
            return Err(ApiError::internal_message(
                "request identity diff returned an unexpected commit",
            ));
        }
        current_oid = Some(oid);
    }
    if summaries.len() != expected.len() {
        return Err(ApiError::internal_message(
            "request identity diff did not inspect every commit",
        ));
    }
    Ok(summaries)
}

fn validate_request_diff_header(field: &BoundedNulField) -> Result<(), ApiError> {
    if field.truncated {
        return Err(ApiError::internal_message(
            "request identity diff header is too large",
        ));
    }
    let header = std::str::from_utf8(&field.bytes).map_err(ApiError::bad_request)?;
    let columns = header.split_ascii_whitespace().collect::<Vec<_>>();
    if columns.len() != 5 || !columns[0].starts_with(':') {
        return Err(ApiError::internal_message(format!(
            "invalid request identity diff header {header}"
        )));
    }
    Ok(())
}

fn request_path_is_public(
    policy: &scope_domain::policy::Policy,
    raw_path: &[u8],
) -> Result<bool, ApiError> {
    let path = std::str::from_utf8(raw_path).map_err(ApiError::bad_request)?;
    let scope_path = ScopePath::parse(format!("/{path}")).map_err(ApiError::bad_request)?;
    Ok(policy.can_read(&scope_path, false))
}

struct BoundedNulField {
    bytes: Vec<u8>,
    truncated: bool,
}

fn read_nul_field_bounded(reader: &mut impl BufRead) -> Result<Option<BoundedNulField>, ApiError> {
    let mut bytes = Vec::new();
    let mut truncated = false;
    loop {
        let available = reader.fill_buf().map_err(|error| {
            ApiError::infrastructure_unavailable(format!(
                "reading request identity diff failed: {error}"
            ))
        })?;
        if available.is_empty() {
            if bytes.is_empty() && !truncated {
                return Ok(None);
            }
            return Err(ApiError::internal_message(
                "request identity diff ended inside a field",
            ));
        }
        let consumed = available
            .iter()
            .position(|byte| *byte == 0)
            .map_or(available.len(), |position| position + 1);
        let content =
            &available[..consumed.saturating_sub(usize::from(available[consumed - 1] == 0))];
        let remaining = MAX_REQUEST_DIFF_FIELD_BYTES.saturating_sub(bytes.len());
        bytes.extend_from_slice(&content[..content.len().min(remaining)]);
        truncated |= content.len() > remaining;
        let terminated = available[consumed - 1] == 0;
        reader.consume(consumed);
        if terminated {
            return Ok(Some(BoundedNulField { bytes, truncated }));
        }
    }
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
    repo: &Repository,
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
