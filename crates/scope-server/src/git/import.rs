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
    http::responses::{first_push_token_status_at, pending_scope_path, repo_owner_ids},
    persistence::{lock_catalog, persist_catalog, unix_now},
    state::AppState,
    state::{find_repo, live_tree},
};
use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64};
use scope_policy::{ScopePath, Visibility, VisibilityRule};
use scope_projection::{AuthorVisibility, FileChange, LogicalCommit, MixedCommitPolicy};
use scope_store::{
    FirstPushTokenStatus, PendingImport, PendingImportFile, RepoPublicationState, StagedFileChange,
    StagedFileChangeKind, StagedRepoUpdate, StoredRepository,
};
use sha2::{Digest, Sha256};
use std::{collections::BTreeMap, path::Path as FsPath, process::Command};

#[derive(Clone, Debug)]
pub(crate) struct ReceivePackFileChange {
    pub(crate) path: ScopePath,
    pub(crate) content: Option<String>,
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
    pub(crate) changes: Vec<ReceivePackFileChange>,
}

// Handoff point for a real post-publish receive-pack parser. This stays
// private so JSON never becomes the product push flow.
#[allow(dead_code)]
pub(crate) fn stage_receive_pack_update(
    repo: &mut StoredRepository,
    update: ReceivePackUpdate,
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

    let staged_update = build_staged_receive_pack_update(repo, update)?;
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
) -> Result<StagedRepoUpdate, ApiError> {
    let live_tree = live_tree(repo);
    let mut staged_changes = Vec::with_capacity(update.changes.len());

    for change in update.changes {
        let old_content = live_tree.get(&change.path).cloned();
        if old_content == change.content {
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
        changes: staged_changes,
    })
}

pub(crate) fn apply_receive_pack_update(
    repo: &mut StoredRepository,
    staged_update: StagedRepoUpdate,
) -> Result<(), ApiError> {
    validate_staged_update_policy(repo, &staged_update)?;
    let owner_ids = repo_owner_ids(repo);
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
                path: change.path,
                old_content: change.old_content,
                new_content: change.new_content,
            })
            .collect(),
    });
    Ok(())
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
    let files = git_tree_files(staging_repo, &head_oid)?;

    Ok(PendingImport {
        default_branch: default_branch.to_string(),
        head_oid,
        tree_oid,
        imported_at_unix: unix_now()?,
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
    let pushed_tree = pushed_scope_tree(staging_repo, &head_oid)?;
    let repo = find_repo(state, owner, repo_name)?;
    if repo.record.publication_state != RepoPublicationState::Published {
        return Err(ApiError::conflict("repo must be published before push"));
    }
    let live_tree = live_tree(&repo);
    let mut changes = Vec::new();

    for (path, new_content) in &pushed_tree {
        if live_tree.get(path) != Some(new_content) {
            changes.push(ReceivePackFileChange {
                path: path.clone(),
                content: Some(new_content.clone()),
            });
        }
    }
    for path in live_tree.keys() {
        if !pushed_tree.contains_key(path) {
            changes.push(ReceivePackFileChange {
                path: path.clone(),
                content: None,
            });
        }
    }

    let message = pushed_commit_message(staging_repo, &head_oid)?;
    Ok(ReceivePackUpdate {
        branch,
        author_id: author_id.to_string(),
        message,
        changes,
    })
}

pub(crate) fn pushed_scope_tree(
    staging_repo: &FsPath,
    head_oid: &str,
) -> Result<BTreeMap<ScopePath, String>, ApiError> {
    let mut tree = BTreeMap::new();
    for file in git_tree_files(staging_repo, head_oid)? {
        let content = BASE64
            .decode(file.content_base64.as_bytes())
            .map_err(ApiError::bad_request)?;
        let content = String::from_utf8(content).map_err(ApiError::bad_request)?;
        tree.insert(pending_scope_path(&file.path)?, content);
    }
    Ok(tree)
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
    staging_repo: &FsPath,
    head_oid: &str,
) -> Result<Vec<PendingImportFile>, ApiError> {
    let output = run_git_output(
        Some(staging_repo),
        &["ls-tree", "-rz", "-r", head_oid],
        "reading pushed tree",
    )?;
    let mut files = Vec::new();
    let mut total_bytes = 0usize;
    for raw in output.stdout.split(|byte| *byte == 0) {
        if raw.is_empty() {
            continue;
        }
        if files.len() >= MAX_PENDING_IMPORT_FILES {
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
        validate_pushed_file_path(path)?;
        if mode != "100644" {
            return Err(ApiError::bad_request(format!(
                "unsupported Git file mode {path}: {mode}"
            )));
        }
        let blob_size = git_stdout_text(
            staging_repo,
            &["cat-file", "-s", oid],
            "reading pushed blob size",
        )?
        .trim()
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
        let content = run_git_output(
            Some(staging_repo),
            &["cat-file", "blob", oid],
            "reading pushed blob",
        )?
        .stdout;
        std::str::from_utf8(&content)
            .map_err(|_| ApiError::bad_request(format!("blob {path} must be valid UTF-8 text")))?;
        files.push(PendingImportFile {
            path: path.to_string(),
            mode: mode.to_string(),
            oid: oid.to_string(),
            content_base64: BASE64.encode(content),
        });
    }
    files.sort_by(|left, right| left.path.cmp(&right.path));
    Ok(files)
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
    let repo_id = scope_store::repo_id(owner, repo_name);
    let now = unix_now()?;
    let mut catalog = lock_catalog(state)?;
    let mut staged = catalog.clone();
    {
        let repo = staged
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
                authorize_first_push_token_for_repo(repo, secret)?;
            }
            InitialPushCredential::GitPushToken { secret } => {
                authorize_git_push_token_for_repo(repo, secret)?;
            }
        }
        if let Some(token) = repo.first_push_token.as_mut() {
            if first_push_token_status_at(token, now) == FirstPushTokenStatus::Active {
                token.used_at_unix = Some(now);
            }
        }
        repo.pending_import = Some(import);
        repo.record.publication_state = RepoPublicationState::PendingPublish;
    }

    persist_catalog(state, &staged)?;
    *catalog = staged;
    Ok(())
}

pub(crate) fn persist_receive_pack_update(
    state: &AppState,
    owner: &str,
    repo_name: &str,
    update: ReceivePackUpdate,
) -> Result<PersistedReceivePackUpdate, ApiError> {
    let repo_id = scope_store::repo_id(owner, repo_name);
    let mut catalog = lock_catalog(state)?;
    let mut staged = catalog.clone();
    let persisted = {
        let repo = staged
            .repositories
            .get_mut(&repo_id)
            .ok_or_else(|| ApiError::not_found(format!("repo {owner}/{repo_name} not found")))?;
        if stage_receive_pack_update(repo, update)?.is_some() {
            PersistedReceivePackUpdate::Staged
        } else {
            PersistedReceivePackUpdate::Applied
        }
    };

    persist_catalog(state, &staged)?;
    *catalog = staged;
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
    let repo_id = scope_store::repo_id(owner, repo_name);
    let digest = Sha256::digest(repo_id.as_bytes());
    format!("repo-{digest:x}")
}
