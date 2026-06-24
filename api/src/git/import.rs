use crate::domain::policy::{ScopePath, Visibility, VisibilityRule};
use crate::domain::projection::{
    AuthorVisibility, FileChange, FileVisibilityChange, LogicalCommit, MixedCommitPolicy,
};
use crate::domain::projection_views::pending_scope_path;
use crate::domain::store::{
    FirstPushTokenStatus, LineDiff, PendingImport, PendingImportFile, RepoPublicationState,
    SourceBlob, StagedFileChange, StagedFileChangeKind, StagedRepoUpdate, StoredRepository,
};
use crate::{
    config::{
        DEFAULT_GIT_BRANCH, MAX_PENDING_IMPORT_BLOB_BYTES, MAX_PENDING_IMPORT_FILES,
        MAX_PENDING_IMPORT_TOTAL_BYTES,
    },
    error::ApiError,
    git::{
        InitialPushCredential, PersistedReceivePackUpdate, authorize_first_push_token_for_repo,
        authorize_git_push_token_for_repo,
    },
    http::responses::repo_owner_ids,
    object_store::{ObjectStore, put_repo_object, source_blob_text},
    persistence::unix_now,
    state::AppState,
    state::{find_repo, live_tree},
};
use sha2::{Digest, Sha256};
use similar::{ChangeTag, TextDiff};
use std::{collections::BTreeSet, path::Path as FsPath, process::Command};

const MAX_EXACT_LINE_DIFF_BYTES: u64 = 1024 * 1024;
const MAX_EXACT_LINE_DIFF_LINES: usize = 20_000;

#[derive(Clone, Debug)]
pub(crate) struct ReceivePackFileChange {
    pub(crate) path: ScopePath,
    pub(crate) content: Option<SourceBlob>,
}

#[allow(dead_code)]
pub(crate) fn ensure_default_branch(branch: &str) -> Result<(), ApiError> {
    let branch = branch.trim();
    match branch {
        DEFAULT_GIT_BRANCH => Ok(()),
        value if value == format!("refs/heads/{DEFAULT_GIT_BRANCH}") => Ok(()),
        value if value.starts_with("refs/tags/") => Err(ApiError::bad_request(
            "tags are not supported by Scope pushes",
        )),
        _ => Err(ApiError::bad_request(
            "Scope accepts pushes only to the default branch refs/heads/main",
        )),
    }
}

#[derive(Clone, Debug)]
#[allow(dead_code)]
pub(crate) struct ReceivePackUpdate {
    pub(crate) branch: String,
    pub(crate) author_id: String,
    pub(crate) message: String,
    pub(crate) git_snapshot: SourceBlob,
    pub(crate) uploaded_blobs: Vec<SourceBlob>,
    pub(crate) changes: Vec<ReceivePackFileChange>,
}

// Handoff point for a real post-publish receive-pack parser. This stays
// private so JSON never becomes the product push flow.
#[allow(dead_code)]
#[cfg(test)]
pub(crate) fn stage_receive_pack_update(
    repo: &mut StoredRepository,
    update: ReceivePackUpdate,
) -> Result<Option<StagedRepoUpdate>, ApiError> {
    let store = crate::object_store::MemoryObjectStore::new();
    stage_receive_pack_update_with_store(repo, update, &store)
}

fn stage_receive_pack_update_with_store(
    repo: &mut StoredRepository,
    update: ReceivePackUpdate,
    store: &dyn ObjectStore,
) -> Result<Option<StagedRepoUpdate>, ApiError> {
    ensure_default_branch(&update.branch)?;
    if repo.record.publication_state != RepoPublicationState::Published {
        return Err(ApiError::conflict("repo must be published before push"));
    }
    if update.changes.is_empty() {
        return Err(ApiError::bad_request(
            "receive-pack update must include file changes",
        ));
    }
    if repo.staged_update.is_some() {
        return Err(ApiError::conflict("a staged update is already pending"));
    }

    let staged_update = build_staged_receive_pack_update(repo, update, store)?;
    if repo.settings.review_pushes_before_applying {
        repo.staged_update = Some(staged_update.clone());
        Ok(Some(staged_update))
    } else {
        apply_receive_pack_update(repo, staged_update)?;
        Ok(None)
    }
}

#[allow(dead_code)]
pub(crate) fn build_staged_receive_pack_update(
    repo: &StoredRepository,
    update: ReceivePackUpdate,
    store: &dyn ObjectStore,
) -> Result<StagedRepoUpdate, ApiError> {
    let live_tree = live_tree(repo);
    let mut staged_changes = Vec::with_capacity(update.changes.len());

    for change in update.changes {
        let old_content = live_tree.get(&change.path).cloned();
        if source_content_matches(old_content.as_ref(), change.content.as_ref()) {
            continue;
        }
        let kind = match (&old_content, &change.content) {
            (None, Some(_)) => StagedFileChangeKind::Added,
            (Some(_), Some(_)) => StagedFileChangeKind::Modified,
            (Some(_), None) => StagedFileChangeKind::Deleted,
            (None, None) => continue,
        };
        let visibility = repo.policy.effective_visibility(&change.path);
        staged_changes.push(StagedFileChange {
            path: change.path,
            line_diff: if repo.settings.review_pushes_before_applying {
                staged_file_line_diff(store, old_content.as_ref(), change.content.as_ref())?
            } else {
                LineDiff::default()
            },
            old_content,
            new_content: change.content,
            visibility,
            kind,
        });
    }

    if staged_changes.is_empty() {
        return Err(ApiError::bad_request(
            "receive-pack update did not change the live tree",
        ));
    }

    Ok(StagedRepoUpdate {
        id: format!("staged_push_{}", repo.graph.commits.len() + 1),
        branch: update.branch,
        base_live_commit_id: repo.graph.commits.last().map(|commit| commit.id.clone()),
        author_id: update.author_id,
        message: update.message,
        git_snapshot: update.git_snapshot,
        changes: staged_changes,
    })
}

fn source_content_matches(left: Option<&SourceBlob>, right: Option<&SourceBlob>) -> bool {
    match (left, right) {
        (Some(left), Some(right)) => {
            left.sha256 == right.sha256
                && left.git_oid == right.git_oid
                && left.size_bytes == right.size_bytes
        }
        (None, None) => true,
        _ => false,
    }
}

fn staged_file_line_diff(
    store: &dyn ObjectStore,
    old_content: Option<&SourceBlob>,
    new_content: Option<&SourceBlob>,
) -> Result<LineDiff, ApiError> {
    match (old_content, new_content) {
        (None, Some(new_content)) => {
            return Ok(LineDiff {
                additions: new_content.line_count,
                deletions: 0,
            });
        }
        (Some(old_content), None) => {
            return Ok(LineDiff {
                additions: 0,
                deletions: old_content.line_count,
            });
        }
        (None, None) => return Ok(LineDiff::default()),
        (Some(old_content), Some(new_content))
            if line_diff_requires_count_fallback(old_content, new_content) =>
        {
            return Ok(LineDiff {
                additions: new_content.line_count,
                deletions: old_content.line_count,
            });
        }
        (Some(_), Some(_)) => {}
    }

    let old_content = old_content
        .map(|blob| source_blob_text(store, blob))
        .transpose()?
        .unwrap_or_default();
    let new_content = new_content
        .map(|blob| source_blob_text(store, blob))
        .transpose()?
        .unwrap_or_default();

    Ok(line_diff_between(&old_content, &new_content))
}

fn line_diff_requires_count_fallback(old_content: &SourceBlob, new_content: &SourceBlob) -> bool {
    [old_content, new_content].iter().any(|blob| {
        blob.size_bytes > MAX_EXACT_LINE_DIFF_BYTES || blob.line_count > MAX_EXACT_LINE_DIFF_LINES
    })
}

fn line_diff_between(old_content: &str, new_content: &str) -> LineDiff {
    let mut line_diff = LineDiff::default();
    let old_lines = old_content.lines().collect::<Vec<_>>();
    let new_lines = new_content.lines().collect::<Vec<_>>();
    let diff = TextDiff::from_slices(&old_lines, &new_lines);
    for change in diff.iter_all_changes() {
        match change.tag() {
            ChangeTag::Delete => line_diff.deletions += 1,
            ChangeTag::Insert => line_diff.additions += 1,
            ChangeTag::Equal => {}
        }
    }
    line_diff
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn line_diff_counts_separate_hunks_without_context() {
        let diff = line_diff_between(
            "one\nold-a\nsame\nold-b\nlast",
            "one\nnew-a\nsame\nnew-b\nlast",
        );

        assert_eq!(diff.deletions, 2);
        assert_eq!(diff.additions, 2);
    }

    #[test]
    fn line_diff_counts_appended_line_without_recounting_existing_line() {
        let diff = line_diff_between("hello", "hello\nnew line");

        assert_eq!(diff.deletions, 0);
        assert_eq!(diff.additions, 1);
    }

    #[test]
    fn staged_file_line_diff_counts_added_file_without_blob_read() {
        let blob = test_source_blob("missing-added", 5, 512);
        let diff = staged_file_line_diff(
            &crate::object_store::MemoryObjectStore::new(),
            None,
            Some(&blob),
        )
        .unwrap();

        assert_eq!(diff.deletions, 0);
        assert_eq!(diff.additions, 5);
    }

    #[test]
    fn staged_file_line_diff_uses_count_fallback_for_large_modified_blobs() {
        let old_blob = test_source_blob("missing-old-large", 8, MAX_EXACT_LINE_DIFF_BYTES + 1);
        let new_blob = test_source_blob("missing-new-large", 13, MAX_EXACT_LINE_DIFF_BYTES + 1);
        let diff = staged_file_line_diff(
            &crate::object_store::MemoryObjectStore::new(),
            Some(&old_blob),
            Some(&new_blob),
        )
        .unwrap();

        assert_eq!(diff.deletions, 8);
        assert_eq!(diff.additions, 13);
    }

    #[test]
    fn line_diff_counts_large_middle_rewrite() {
        let old_content = (0..10_000)
            .map(|index| format!("same-{index}"))
            .chain(["old middle".to_string()])
            .chain((0..10_000).map(|index| format!("tail-{index}")))
            .collect::<Vec<_>>()
            .join("\n");
        let new_content = (0..10_000)
            .map(|index| format!("same-{index}"))
            .chain(["new middle".to_string()])
            .chain((0..10_000).map(|index| format!("tail-{index}")))
            .collect::<Vec<_>>()
            .join("\n");

        let diff = line_diff_between(&old_content, &new_content);

        assert_eq!(diff.deletions, 1);
        assert_eq!(diff.additions, 1);
    }

    fn test_source_blob(label: &str, line_count: usize, size_bytes: u64) -> SourceBlob {
        SourceBlob {
            object_key: format!("objects/test/{label}"),
            sha256: format!("sha256-{label}"),
            git_oid: format!("oid-{label}"),
            size_bytes,
            line_count,
        }
    }
}

pub(crate) fn apply_receive_pack_update(
    repo: &mut StoredRepository,
    staged_update: StagedRepoUpdate,
) -> Result<(), ApiError> {
    validate_staged_update_policy(repo, &staged_update)?;
    let owner_ids = repo_owner_ids(repo);
    let visibility_changes = staged_update
        .changes
        .iter()
        .filter(|change| change.new_content.is_some())
        .filter_map(|change| {
            let old_visibility = repo.policy.effective_visibility(&change.path);
            if old_visibility == Visibility::Public
                && change.visibility == Visibility::Private
                && change.old_content.is_some()
            {
                Some(FileVisibilityChange {
                    path: change.path.clone(),
                    old_visibility,
                    new_visibility: change.visibility,
                    current_content: change.new_content.clone(),
                })
            } else {
                None
            }
        })
        .collect::<Vec<_>>();
    for change in &staged_update.changes {
        if change.new_content.is_none() {
            continue;
        }

        let rule = staged_visibility_rule(change, &owner_ids);
        repo.policy.add_rule(rule).map_err(ApiError::bad_request)?;
    }

    let parent_ids = repo
        .graph
        .commits
        .last()
        .map(|commit| vec![commit.id.clone()])
        .unwrap_or_default();
    repo.graph.commits.push(LogicalCommit {
        id: format!("rv_push_{}", repo.graph.commits.len() + 1),
        parent_ids,
        author_id: staged_update.author_id,
        author_visibility: AuthorVisibility::Visible,
        message: staged_update.message,
        mixed_policy: MixedCommitPolicy::SyntheticPublicCommit,
        changes: staged_update
            .changes
            .into_iter()
            .map(|change| FileChange {
                visibility: applied_file_visibility(repo, &change),
                path: change.path,
                old_content: change.old_content,
                new_content: change.new_content,
            })
            .collect(),
        visibility_changes,
    });
    repo.git_snapshot = Some(staged_update.git_snapshot);
    Ok(())
}

fn applied_file_visibility(repo: &StoredRepository, change: &StagedFileChange) -> Visibility {
    if change.new_content.is_none() {
        repo.policy.effective_visibility(&change.path)
    } else {
        change.visibility
    }
}

pub(crate) fn validate_staged_update_policy(
    repo: &StoredRepository,
    staged_update: &StagedRepoUpdate,
) -> Result<(), ApiError> {
    let owner_ids = repo_owner_ids(repo);
    let mut policy = repo.policy.clone();
    for change in &staged_update.changes {
        if change.new_content.is_none() {
            continue;
        }

        let rule = staged_visibility_rule(change, &owner_ids);
        policy.add_rule(rule).map_err(ApiError::bad_request)?;
    }

    Ok(())
}

pub(crate) fn staged_visibility_rule(
    change: &StagedFileChange,
    owner_ids: &[String],
) -> VisibilityRule {
    match change.visibility {
        Visibility::Public => VisibilityRule::public(change.path.clone()),
        Visibility::Private => VisibilityRule::private(change.path.clone(), owner_ids.to_vec()),
    }
}
pub(crate) fn pending_import_from_staging_repo(
    state: &AppState,
    owner: &str,
    repo_name: &str,
    staging_repo: &FsPath,
) -> Result<PendingImport, ApiError> {
    let refs = git_refs(staging_repo)?;
    if refs.len() != 1 {
        return Err(ApiError::bad_request(format!(
            "push must create exactly one branch and no tags; found {}",
            describe_refs(&refs)
        )));
    }
    let (refname, head_oid) = refs.into_iter().next().expect("length checked");
    let Some(default_branch) = refname.strip_prefix("refs/heads/") else {
        return Err(ApiError::bad_request("only branch pushes are supported"));
    };
    ensure_default_branch(default_branch)?;
    let tree_oid = git_stdout_text(
        staging_repo,
        &["rev-parse", &format!("{head_oid}^{{tree}}")],
        "reading pushed tree",
    )?
    .trim()
    .to_string();
    let imported_at_unix = unix_now()?;
    let repo_id = crate::domain::store::repo_id(owner, repo_name);
    let files = git_tree_files(state, &repo_id, staging_repo, &head_oid)?;
    let uploaded_file_blobs = files
        .iter()
        .map(|file| file.blob.clone())
        .collect::<Vec<_>>();
    let git_snapshot = match git_snapshot_from_repo(state, &repo_id, staging_repo) {
        Ok(snapshot) => snapshot,
        Err(error) => {
            crate::state::best_effort_cleanup_rollback_source_blobs(state, &uploaded_file_blobs);
            return Err(error);
        }
    };

    Ok(PendingImport {
        default_branch: default_branch.to_string(),
        head_oid,
        tree_oid,
        imported_at_unix,
        git_snapshot,
        files,
    })
}

pub(crate) fn receive_pack_update_from_staging_repo(
    state: &AppState,
    owner: &str,
    repo_name: &str,
    staging_repo: &FsPath,
    author_id: &str,
) -> Result<ReceivePackUpdate, ApiError> {
    let refs = git_refs(staging_repo)?;
    if refs.len() != 1 {
        return Err(ApiError::bad_request(format!(
            "push must update exactly one branch and no tags; found {}",
            describe_refs(&refs)
        )));
    }
    let (branch, head_oid) = refs.into_iter().next().expect("length checked");
    ensure_default_branch(&branch)?;
    let repo = find_repo(state, owner, repo_name)?;
    if repo.record.publication_state != RepoPublicationState::Published {
        return Err(ApiError::conflict("repo must be published before push"));
    }
    let repo_id = crate::domain::store::repo_id(owner, repo_name);
    let message = pushed_commit_message(staging_repo, &head_oid)?;
    let live_tree = live_tree(&repo);
    let pushed_entries = git_tree_entries(staging_repo, &head_oid)?;
    let mut changes = Vec::new();
    let mut uploaded_file_blobs = Vec::new();
    let mut pushed_paths = BTreeSet::new();

    for entry in pushed_entries {
        let path = match pending_scope_path(&entry.path) {
            Ok(path) => path,
            Err(error) => {
                crate::state::best_effort_cleanup_rollback_source_blobs(
                    state,
                    &uploaded_file_blobs,
                );
                return Err(error);
            }
        };
        pushed_paths.insert(path.clone());
        let live_content = live_tree.get(&path);
        if live_content.is_some_and(|blob| {
            blob.git_oid == entry.oid && blob.size_bytes == entry.size_bytes as u64
        }) {
            continue;
        }

        let content = match read_git_tree_blob(staging_repo, &entry.oid, &entry.path) {
            Ok(content) => content,
            Err(error) => {
                crate::state::best_effort_cleanup_rollback_source_blobs(
                    state,
                    &uploaded_file_blobs,
                );
                return Err(error);
            }
        };
        let new_content =
            match put_repo_object(state.object_store.as_ref(), &repo_id, "blobs", &content) {
                Ok(blob) => blob,
                Err(error) => {
                    crate::state::best_effort_cleanup_rollback_source_blobs(
                        state,
                        &uploaded_file_blobs,
                    );
                    return Err(error);
                }
            };
        uploaded_file_blobs.push(new_content.clone());
        if !source_content_matches(live_content, Some(&new_content)) {
            changes.push(ReceivePackFileChange {
                path,
                content: Some(new_content),
            });
        }
    }
    for path in live_tree.keys() {
        if !pushed_paths.contains(path) {
            changes.push(ReceivePackFileChange {
                path: path.clone(),
                content: None,
            });
        }
    }
    if changes.is_empty() {
        return Err(ApiError::bad_request(
            "receive-pack update did not change the live tree",
        ));
    }
    let git_snapshot = match git_snapshot_from_repo(state, &repo_id, staging_repo) {
        Ok(snapshot) => snapshot,
        Err(error) => {
            crate::state::best_effort_cleanup_rollback_source_blobs(state, &uploaded_file_blobs);
            return Err(error);
        }
    };

    Ok(ReceivePackUpdate {
        branch,
        author_id: author_id.to_string(),
        message,
        git_snapshot,
        uploaded_blobs: uploaded_file_blobs,
        changes,
    })
}

pub(crate) fn pushed_commit_message(
    staging_repo: &FsPath,
    head_oid: &str,
) -> Result<String, ApiError> {
    let message = git_stdout_text(
        staging_repo,
        &["log", "-1", "--format=%B", head_oid],
        "reading pushed commit message",
    )?;
    let message = message.trim_end_matches(&['\r', '\n'][..]).to_string();
    if message.trim().is_empty() {
        Ok(format!("Push to {DEFAULT_GIT_BRANCH}"))
    } else {
        Ok(message)
    }
}

pub(crate) fn git_refs(staging_repo: &FsPath) -> Result<Vec<(String, String)>, ApiError> {
    let output = run_git_output(
        Some(staging_repo),
        &[
            "for-each-ref",
            "--format=%(refname)%00%(objectname)",
            "refs/heads",
            "refs/tags",
        ],
        "reading pushed refs",
    )?;
    if !output.status.success() {
        return Err(ApiError::service_unavailable(format!(
            "reading pushed refs: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        )));
    }
    let text = String::from_utf8(output.stdout).map_err(ApiError::bad_request)?;
    text.lines()
        .map(|line| {
            let (refname, oid) = line
                .split_once('\0')
                .ok_or_else(|| ApiError::internal_message("invalid git ref listing"))?;
            Ok((refname.to_string(), oid.to_string()))
        })
        .collect()
}

pub(crate) fn describe_refs(refs: &[(String, String)]) -> String {
    if refs.is_empty() {
        return "none".to_string();
    }

    refs.iter()
        .map(|(name, oid)| format!("{name}@{}", oid.get(..12).unwrap_or(oid)))
        .collect::<Vec<_>>()
        .join(", ")
}

pub(crate) fn git_tree_files(
    state: &AppState,
    repo_id: &str,
    staging_repo: &FsPath,
    head_oid: &str,
) -> Result<Vec<PendingImportFile>, ApiError> {
    let pending_files = git_tree_entries(staging_repo, head_oid)?;
    let mut files = Vec::with_capacity(pending_files.len());
    let mut uploaded_blobs = Vec::with_capacity(pending_files.len());
    for pending in pending_files {
        let content = match read_git_tree_blob(staging_repo, &pending.oid, &pending.path) {
            Ok(content) => content,
            Err(error) => {
                crate::state::best_effort_cleanup_rollback_source_blobs(state, &uploaded_blobs);
                return Err(error);
            }
        };
        let blob = match put_repo_object(state.object_store.as_ref(), repo_id, "blobs", &content) {
            Ok(blob) => blob,
            Err(error) => {
                crate::state::best_effort_cleanup_rollback_source_blobs(state, &uploaded_blobs);
                return Err(error);
            }
        };
        uploaded_blobs.push(blob.clone());
        files.push(PendingImportFile {
            path: pending.path,
            mode: pending.mode,
            oid: pending.oid,
            blob,
        });
    }
    files.sort_by(|left, right| left.path.cmp(&right.path));
    Ok(files)
}

fn git_tree_entries(
    staging_repo: &FsPath,
    head_oid: &str,
) -> Result<Vec<PendingGitTreeFile>, ApiError> {
    let output = run_git_output(
        Some(staging_repo),
        &["ls-tree", "-rz", "-r", "-l", head_oid],
        "reading pushed tree",
    )?;
    let mut pending_files = Vec::new();
    let mut total_bytes = 0usize;
    for raw in output.stdout.split(|byte| *byte == 0) {
        if raw.is_empty() {
            continue;
        }
        if pending_files.len() >= MAX_PENDING_IMPORT_FILES {
            return Err(ApiError::bad_request(format!(
                "pending import exceeds {MAX_PENDING_IMPORT_FILES} files"
            )));
        }
        let entry = std::str::from_utf8(raw).map_err(ApiError::bad_request)?;
        let Some((metadata, path)) = entry.split_once('\t') else {
            return Err(ApiError::internal_message("invalid git tree entry"));
        };
        let mut fields = metadata.split_whitespace();
        let mode = fields
            .next()
            .ok_or_else(|| ApiError::internal_message("tree entry is missing mode"))?;
        let kind = fields
            .next()
            .ok_or_else(|| ApiError::internal_message("tree entry is missing type"))?;
        let oid = fields
            .next()
            .ok_or_else(|| ApiError::internal_message("tree entry is missing oid"))?;
        if kind != "blob" {
            return Err(ApiError::bad_request(format!(
                "unsupported Git tree entry {path}: {kind}"
            )));
        }
        let size = fields
            .next()
            .ok_or_else(|| ApiError::internal_message("tree entry is missing size"))?;
        validate_pushed_file_path(path)?;
        if mode != "100644" {
            return Err(ApiError::bad_request(format!(
                "unsupported Git file mode {path}: {mode}"
            )));
        }
        let blob_size = size
            .parse::<usize>()
            .map_err(|_| ApiError::internal_message("invalid Git blob size"))?;
        if blob_size > MAX_PENDING_IMPORT_BLOB_BYTES {
            return Err(ApiError::bad_request(format!(
                "blob {path} is larger than {MAX_PENDING_IMPORT_BLOB_BYTES} bytes"
            )));
        }
        total_bytes = total_bytes
            .checked_add(blob_size)
            .ok_or_else(|| ApiError::bad_request("pending import is too large"))?;
        if total_bytes > MAX_PENDING_IMPORT_TOTAL_BYTES {
            return Err(ApiError::bad_request(format!(
                "pending import exceeds {MAX_PENDING_IMPORT_TOTAL_BYTES} bytes"
            )));
        }
        pending_files.push(PendingGitTreeFile {
            path: path.to_string(),
            mode: mode.to_string(),
            oid: oid.to_string(),
            size_bytes: blob_size,
        });
    }

    Ok(pending_files)
}

fn read_git_tree_blob(staging_repo: &FsPath, oid: &str, path: &str) -> Result<Vec<u8>, ApiError> {
    let content = run_git_output(
        Some(staging_repo),
        &["cat-file", "blob", oid],
        "reading pushed blob",
    )?
    .stdout;
    std::str::from_utf8(&content)
        .map_err(|_| ApiError::bad_request(format!("blob {path} must be valid UTF-8 text")))?;
    Ok(content)
}

pub(crate) fn git_snapshot_from_repo(
    state: &AppState,
    repo_id: &str,
    repo: &FsPath,
) -> Result<SourceBlob, ApiError> {
    let bundle_path = repo.join(format!(
        "scope-snapshot-{}-{}.bundle",
        std::process::id(),
        unix_now()?
    ));
    let bundle = bundle_path.to_string_lossy().to_string();
    run_git(
        Some(repo),
        &["bundle", "create", &bundle, "--all"],
        "creating Git snapshot bundle",
    )?;
    let bytes = std::fs::read(&bundle_path).map_err(ApiError::internal)?;
    let _ = std::fs::remove_file(&bundle_path);
    put_repo_object(state.object_store.as_ref(), repo_id, "git-bundles", &bytes)
}

struct PendingGitTreeFile {
    path: String,
    mode: String,
    oid: String,
    size_bytes: usize,
}

pub(crate) fn validate_pushed_file_path(path: &str) -> Result<(), ApiError> {
    if path.is_empty() || path.starts_with('/') || path.contains('\\') {
        return Err(ApiError::bad_request(format!(
            "unsupported Git file path {path:?}"
        )));
    }
    if path.bytes().any(|byte| byte < 0x20 || byte == 0x7f) {
        return Err(ApiError::bad_request(format!(
            "unsupported Git file path {path:?}"
        )));
    }

    let scope_path = ScopePath::parse(format!("/{path}")).map_err(ApiError::bad_request)?;
    if scope_path.as_str() != format!("/{path}") {
        return Err(ApiError::bad_request(format!(
            "unsupported Git file path {path:?}"
        )));
    }

    Ok(())
}

pub(crate) fn persist_pending_import(
    state: &AppState,
    owner: &str,
    repo_name: &str,
    credential: &InitialPushCredential,
    import: PendingImport,
) -> Result<(), ApiError> {
    let repo_id = crate::domain::store::repo_id(owner, repo_name);
    let now = unix_now()?;
    let owner = owner.to_string();
    let repo_name = repo_name.to_string();
    let credential = credential.clone();
    state.metadata.update(move |catalog| {
        let repo = catalog
            .repositories
            .get_mut(&repo_id)
            .ok_or_else(|| ApiError::not_found(format!("repo {owner}/{repo_name} not found")))?;
        if repo.record.publication_state != RepoPublicationState::PendingFirstPush {
            return Err(ApiError::conflict(
                "repo is not waiting for an initial Git push",
            ));
        }
        if repo.pending_import.is_some() {
            return Err(ApiError::conflict("repo already has a pending import"));
        }
        match credential {
            InitialPushCredential::FirstPushToken { secret } => {
                authorize_first_push_token_for_repo(repo, &secret)?;
            }
            InitialPushCredential::GitPushToken { secret } => {
                authorize_git_push_token_for_repo(repo, &secret)?;
            }
        }
        if let Some(token) = repo.first_push_token.as_mut() {
            if token.status_at(now) == FirstPushTokenStatus::Active {
                token.used_at_unix = Some(now);
            }
        }
        repo.pending_import = Some(import);
        repo.record.publication_state = RepoPublicationState::PendingPublish;
        Ok(())
    })
}

#[cfg(test)]
pub(crate) fn persist_receive_pack_update(
    state: &AppState,
    owner: &str,
    repo_name: &str,
    update: ReceivePackUpdate,
) -> Result<PersistedReceivePackUpdate, ApiError> {
    let repo_id = crate::domain::store::repo_id(owner, repo_name);
    let owner = owner.to_string();
    let repo_name = repo_name.to_string();
    let store = state.object_store.clone();
    state.metadata.update(move |catalog| {
        let repo = catalog
            .repositories
            .get_mut(&repo_id)
            .ok_or_else(|| ApiError::not_found(format!("repo {owner}/{repo_name} not found")))?;
        if stage_receive_pack_update_with_store(repo, update, store.as_ref())?.is_some() {
            Ok(PersistedReceivePackUpdate::Staged)
        } else {
            Ok(PersistedReceivePackUpdate::Applied)
        }
    })
}

pub(crate) fn persist_receive_pack_update_and_promote(
    state: &AppState,
    owner: &str,
    repo_name: &str,
    update: ReceivePackUpdate,
) -> Result<PersistedReceivePackUpdate, ApiError> {
    let repo_id = crate::domain::store::repo_id(owner, repo_name);
    let owner = owner.to_string();
    let repo_name = repo_name.to_string();
    let uploaded_blobs = update.uploaded_blobs.clone();
    let store = state.object_store.clone();

    let persisted = state.metadata.update(move |catalog| {
        let repo = catalog
            .repositories
            .get_mut(&repo_id)
            .ok_or_else(|| ApiError::not_found(format!("repo {owner}/{repo_name} not found")))?;
        let old_snapshot = repo.git_snapshot.clone();
        let persisted =
            if stage_receive_pack_update_with_store(repo, update, store.as_ref())?.is_some() {
                crate::state::queue_source_blob_deletions(catalog, uploaded_blobs);
                PersistedReceivePackUpdate::Staged
            } else {
                let mut cleanup_blobs = uploaded_blobs;
                cleanup_blobs.extend(old_snapshot);
                crate::state::queue_source_blob_deletions(catalog, cleanup_blobs);
                PersistedReceivePackUpdate::Applied
            };
        Ok(persisted)
    })?;
    crate::state::best_effort_drain_pending_source_blob_deletions(state);
    Ok(persisted)
}

pub(crate) fn run_git(repo: Option<&FsPath>, args: &[&str], action: &str) -> Result<(), ApiError> {
    let output = run_git_output(repo, args, action)?;
    if output.status.success() {
        Ok(())
    } else {
        Err(ApiError::service_unavailable(format!(
            "{action}: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        )))
    }
}

pub(crate) fn git_stdout_text(
    repo: &FsPath,
    args: &[&str],
    action: &str,
) -> Result<String, ApiError> {
    let output = run_git_output(Some(repo), args, action)?;
    if !output.status.success() {
        return Err(ApiError::service_unavailable(format!(
            "{action}: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        )));
    }
    String::from_utf8(output.stdout).map_err(ApiError::bad_request)
}

pub(crate) fn run_git_output(
    repo: Option<&FsPath>,
    args: &[&str],
    action: &str,
) -> Result<std::process::Output, ApiError> {
    let mut command = Command::new("git");
    if let Some(repo) = repo {
        command.arg("-C").arg(repo);
    }
    command
        .args(args)
        .output()
        .map_err(|error| ApiError::service_unavailable(format!("failed {action}: {error}")))
}

pub(crate) fn safe_repo_key(owner: &str, repo_name: &str) -> String {
    let repo_id = crate::domain::store::repo_id(owner, repo_name);
    let digest = Sha256::digest(repo_id.as_bytes());
    format!("repo-{digest:x}")
}
