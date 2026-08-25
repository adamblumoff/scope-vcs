use super::{DiscussionAnchorInput, MutationContext};
use crate::{
    error::ApiError,
    git::{import::run_git_output, request_refs::with_request_revision_store_repo},
    state::AppState,
};
use scope_domain::{
    policy::ScopePath,
    repository::{Repository, access::RepositoryAccess},
    requests::{RequestDiscussionAnchor, RequestRevision},
};
use std::{collections::BTreeSet, path::Path as FsPath};

pub(super) async fn validate(
    state: &AppState,
    owner: &str,
    repo_name: &str,
    context: &MutationContext,
    anchor: DiscussionAnchorInput,
) -> Result<RequestDiscussionAnchor, ApiError> {
    if anchor.path.is_some() && anchor.commit_oid.is_none() {
        return Err(ApiError::bad_request(
            "request discussion path requires a commit",
        ));
    }
    let revision = state
        .metadata
        .requests()
        .request_revision(&context.request.id, &anchor.revision_id)
        .await?
        .ok_or_else(|| ApiError::not_found("request revision not found"))?;
    let path = anchor
        .path
        .map(|path| normalized_scope_path(&path))
        .transpose()?;
    let commit_oid = anchor.commit_oid.map(canonical_git_oid).transpose()?;
    if let Some(commit_oid) = commit_oid.as_deref() {
        let visible_paths = with_request_revision_store_repo(
            state,
            owner,
            repo_name,
            &context.request,
            &revision,
            |raw_repo| {
                visible_commit_paths(
                    raw_repo,
                    &context.repo,
                    context.access,
                    &revision,
                    commit_oid,
                )
            },
        )?;
        if let Some(path) = path.as_ref()
            && !visible_paths.contains(path)
        {
            return Err(ApiError::bad_request(
                "request discussion path is not changed by the selected commit",
            ));
        }
    }
    Ok(RequestDiscussionAnchor {
        revision_id: revision.id,
        commit_oid,
        path,
    })
}

pub(super) async fn visible_commits(
    state: &AppState,
    owner: &str,
    repo_name: &str,
    context: &MutationContext,
    anchor: Option<&RequestDiscussionAnchor>,
) -> BTreeSet<(String, String)> {
    let Some(anchor) = anchor else {
        return BTreeSet::new();
    };
    let Some(commit_oid) = anchor.commit_oid.as_deref() else {
        return BTreeSet::new();
    };
    if context.access.can_read_private_files {
        return BTreeSet::from([(anchor.revision_id.clone(), commit_oid.to_string())]);
    }
    let result = async {
        let revision = state
            .metadata
            .requests()
            .request_revision(&context.request.id, &anchor.revision_id)
            .await?
            .ok_or_else(|| ApiError::not_found("request revision not found"))?;
        let visible = with_request_revision_store_repo(
            state,
            owner,
            repo_name,
            &context.request,
            &revision,
            |raw_repo| {
                commit_is_fully_visible(
                    raw_repo,
                    &context.repo,
                    context.access,
                    &revision,
                    commit_oid,
                )
            },
        )?;
        Ok::<_, ApiError>(visible)
    }
    .await;
    match result {
        Ok(true) => BTreeSet::from([(anchor.revision_id.clone(), commit_oid.to_string())]),
        Ok(false) => BTreeSet::new(),
        Err(error) => {
            tracing::warn!(
                request_id = %context.request.id,
                revision_id = %anchor.revision_id,
                error = ?error,
                "redacting discussion anchor because request revision inspection failed"
            );
            BTreeSet::new()
        }
    }
}

fn visible_commit_paths(
    raw_repo: &FsPath,
    repo: &Repository,
    access: RepositoryAccess,
    revision: &RequestRevision,
    commit_oid: &str,
) -> Result<BTreeSet<ScopePath>, ApiError> {
    if !commit_belongs_to_revision(raw_repo, revision, commit_oid)? {
        return Err(ApiError::not_found("request revision commit not found"));
    }
    let (paths, has_hidden) = commit_paths(raw_repo, repo, access, commit_oid)?;
    if has_hidden {
        return Err(ApiError::not_found("request revision commit not found"));
    }
    Ok(paths)
}

fn commit_is_fully_visible(
    raw_repo: &FsPath,
    repo: &Repository,
    access: RepositoryAccess,
    revision: &RequestRevision,
    commit_oid: &str,
) -> Result<bool, ApiError> {
    if !commit_belongs_to_revision(raw_repo, revision, commit_oid)? {
        return Ok(false);
    }
    commit_paths(raw_repo, repo, access, commit_oid).map(|(_, has_hidden)| !has_hidden)
}

fn commit_paths(
    raw_repo: &FsPath,
    repo: &Repository,
    access: RepositoryAccess,
    commit_oid: &str,
) -> Result<(BTreeSet<ScopePath>, bool), ApiError> {
    let parents = run_git_output(
        Some(raw_repo),
        &["show", "-s", "--format=%P", commit_oid],
        "reading request commit identity",
    )?;
    if !parents.status.success() {
        return Err(ApiError::infrastructure_unavailable(format!(
            "reading request commit identity: {}",
            String::from_utf8_lossy(&parents.stderr).trim()
        )));
    }
    let parent = String::from_utf8(parents.stdout)
        .map_err(ApiError::bad_request)?
        .split_whitespace()
        .next()
        .map(str::to_string)
        .ok_or_else(|| ApiError::conflict("request revision commit must have a parent"))?;
    let output = run_git_output(
        Some(raw_repo),
        &[
            "--literal-pathspecs",
            "diff",
            "--raw",
            "-z",
            "--no-renames",
            "--abbrev=64",
            &parent,
            commit_oid,
            "--",
        ],
        "reading request changes",
    )?;
    if !output.status.success() {
        return Err(ApiError::infrastructure_unavailable(format!(
            "reading request changes: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        )));
    }
    let mut fields = output.stdout.split(|byte| *byte == 0);
    let mut visible = BTreeSet::new();
    let mut has_hidden = false;
    while let Some(header) = fields.next() {
        if header.is_empty() {
            continue;
        }
        let header = std::str::from_utf8(header).map_err(ApiError::bad_request)?;
        let columns = header.split_ascii_whitespace().collect::<Vec<_>>();
        if columns.len() != 5 || !columns[0].starts_with(':') {
            return Err(ApiError::internal_message(format!(
                "invalid request diff header {header}"
            )));
        }
        let status = columns[4].as_bytes();
        if !matches!(status.first(), Some(b'A' | b'M' | b'T' | b'D')) {
            return Err(ApiError::internal_message(format!(
                "unsupported request diff status {}",
                String::from_utf8_lossy(status)
            )));
        }
        let path = fields
            .next()
            .ok_or_else(|| ApiError::internal_message("request diff is missing a path"))?;
        let path = String::from_utf8(path.to_vec()).map_err(ApiError::bad_request)?;
        let path = ScopePath::parse(format!("/{path}")).map_err(ApiError::bad_request)?;
        if repo.policy.can_read(&path, access.can_read_private_files) {
            visible.insert(path);
        } else {
            has_hidden = true;
        }
    }
    Ok((visible, has_hidden))
}

fn commit_belongs_to_revision(
    raw_repo: &FsPath,
    revision: &RequestRevision,
    commit_oid: &str,
) -> Result<bool, ApiError> {
    if !git_commit_exists(raw_repo, commit_oid)? {
        return Ok(false);
    }
    if !git_is_ancestor(raw_repo, commit_oid, &revision.new_head_oid)? {
        return Ok(false);
    }
    Ok(!git_is_ancestor(
        raw_repo,
        commit_oid,
        &revision.old_head_oid,
    )?)
}

fn git_commit_exists(raw_repo: &FsPath, commit_oid: &str) -> Result<bool, ApiError> {
    let commit_object = format!("{commit_oid}^{{commit}}");
    let output = run_git_output(
        Some(raw_repo),
        &["cat-file", "-e", &commit_object],
        "validating request revision commit",
    )?;
    match output.status.code() {
        Some(0) => Ok(true),
        Some(1 | 128) => Ok(false),
        _ => Err(ApiError::infrastructure_unavailable(format!(
            "validating request revision commit: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ))),
    }
}

fn git_is_ancestor(raw_repo: &FsPath, ancestor: &str, descendant: &str) -> Result<bool, ApiError> {
    let output = run_git_output(
        Some(raw_repo),
        &["merge-base", "--is-ancestor", ancestor, descendant],
        "validating request revision commit",
    )?;
    match output.status.code() {
        Some(0) => Ok(true),
        Some(1) => Ok(false),
        _ => Err(ApiError::infrastructure_unavailable(format!(
            "validating request revision commit: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ))),
    }
}

fn normalized_scope_path(path: &str) -> Result<ScopePath, ApiError> {
    let path = ScopePath::parse(format!("/{}", path.trim_start_matches('/')))
        .map_err(ApiError::bad_request)?;
    if path == ScopePath::root() {
        return Err(ApiError::bad_request("file path is required"));
    }
    Ok(path)
}

fn canonical_git_oid(oid: String) -> Result<String, ApiError> {
    if oid.len() != 40 || !oid.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(ApiError::bad_request(
            "Git OID must be exactly 40 hexadecimal characters",
        ));
    }
    Ok(oid.to_ascii_lowercase())
}
