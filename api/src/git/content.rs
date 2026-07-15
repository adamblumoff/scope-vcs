use crate::domain::store::{DEFAULT_GIT_FILE_MODE, SourceBlob};
use crate::{
    error::ApiError,
    git::{import::run_git_output, storage::cached_raw_git_repo},
    object_store::source_blob_bytes,
    state::AppState,
};

use scope_core::git_segments::{
    GIT_BLOB_REFERENCE_PREFIX, GIT_MANIFEST_OBJECT_PREFIX,
    git_blob_reference as segment_git_blob_reference,
};

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
) -> Result<Vec<u8>, ApiError> {
    let Some(reference) = blob.object_key.strip_prefix(GIT_BLOB_REFERENCE_PREFIX) else {
        return Ok(source_blob_bytes(state.object_store.as_ref(), blob)?);
    };
    let (manifest_id, manifest_sha256) = reference
        .split_once('/')
        .ok_or_else(|| ApiError::internal_message("invalid Git blob reference"))?;
    let snapshot = SourceBlob {
        object_key: format!("{GIT_MANIFEST_OBJECT_PREFIX}{manifest_id}"),
        sha256: manifest_sha256.to_string(),
        git_oid: String::new(),
        git_file_mode: DEFAULT_GIT_FILE_MODE.to_string(),
        size_bytes: 0,
    };
    let repo = cached_raw_git_repo(state, &snapshot)?;
    let output = run_git_output(
        Some(&repo),
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
