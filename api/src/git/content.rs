use crate::{error::ApiError, git::import::run_git_output, state::AppState};
use scope_domain::{
    content_ref::ContentRef,
    store::{GitHead, GitPackSpan, SourceBlob},
};
use scope_git::git_blob_reference as segment_git_blob_reference;
use scope_object_store::source_blob_bytes;
use std::path::Path;

pub(crate) fn git_blob_reference(
    snapshot: &SourceBlob,
    oid: String,
    mode: String,
    size_bytes: usize,
) -> Result<SourceBlob, ApiError> {
    Ok(segment_git_blob_reference(
        snapshot,
        oid,
        mode,
        size_bytes as u64,
    )?)
}

pub(crate) fn source_content_bytes(
    state: &AppState,
    blob: &SourceBlob,
    git_source: Option<(&str, &GitHead, &[GitPackSpan])>,
) -> Result<Vec<u8>, ApiError> {
    if !matches!(blob.content_ref, ContentRef::GitBlob { .. }) {
        return Ok(source_blob_bytes(state.object_store.as_ref(), blob)?);
    }
    let (repository_id, head, pack_spans) = git_source.ok_or_else(|| {
        ApiError::internal_message("Git blob content requires a current pack layout")
    })?;
    if !matches!(head.manifest.content_ref, ContentRef::GitManifestSha256(_)) {
        return Err(ApiError::internal_message(
            "Git blob content locator must be a Git manifest",
        ));
    }
    let repo =
        state
            .repository_engine
            .materialize_repository(state, repository_id, head, pack_spans)?;
    source_content_bytes_from_repo(state, blob, Some(&repo))
}

pub(crate) fn source_content_bytes_from_repo(
    state: &AppState,
    blob: &SourceBlob,
    git_repo: Option<&Path>,
) -> Result<Vec<u8>, ApiError> {
    let ContentRef::GitBlob {
        git_oid: content_oid,
    } = &blob.content_ref
    else {
        return Ok(source_blob_bytes(state.object_store.as_ref(), blob)?);
    };
    if content_oid != &blob.git_oid {
        return Err(ApiError::internal_message(
            "Git blob identity does not match persisted OID",
        ));
    }
    let repo = git_repo.ok_or_else(|| {
        ApiError::internal_message("Git blob content requires a materialized source repository")
    })?;
    let output = run_git_output(
        Some(repo),
        &["cat-file", "blob", &blob.git_oid],
        "reading Git blob content",
    )?;
    if output.stdout.len() as u64 != blob.size_bytes {
        return Err(ApiError::internal_message(format!(
            "Git blob {} size did not match persisted metadata",
            blob.git_oid
        )));
    }
    Ok(output.stdout)
}
