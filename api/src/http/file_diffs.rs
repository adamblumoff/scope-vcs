use crate::{
    domain::store::{SourceBlob, StagedFileChangeKind},
    error::ApiError,
    http::responses::{ReviewFileContentResponse, ReviewFileDiffResponse},
    object_store::{ObjectStore, source_blob_bytes},
};

const MAX_RENDERED_TEXT_BYTES: usize = 1024 * 1024;

pub(crate) fn review_file_diff_response_for_blobs(
    store: &dyn ObjectStore,
    path: String,
    kind: StagedFileChangeKind,
    old_content: Option<&SourceBlob>,
    new_content: Option<&SourceBlob>,
) -> Result<ReviewFileDiffResponse, ApiError> {
    Ok(ReviewFileDiffResponse {
        path,
        kind,
        old_content: old_content
            .map(|blob| review_content_response_for_blob(store, blob))
            .transpose()?,
        new_content: new_content
            .map(|blob| review_content_response_for_blob(store, blob))
            .transpose()?,
    })
}

fn review_content_response_for_blob(
    store: &dyn ObjectStore,
    blob: &SourceBlob,
) -> Result<ReviewFileContentResponse, ApiError> {
    if nonrenderable_blob(blob) {
        return Ok(binary_content(blob));
    }

    let bytes = source_blob_bytes(store, blob)?;
    Ok(review_content_from_bytes(blob, &bytes))
}

fn review_content_from_bytes(blob: &SourceBlob, bytes: &[u8]) -> ReviewFileContentResponse {
    if bytes.len() <= MAX_RENDERED_TEXT_BYTES
        && let Ok(text) = std::str::from_utf8(bytes)
    {
        return ReviewFileContentResponse::Text {
            text: text.to_string(),
        };
    }

    binary_content(blob)
}

fn binary_content(blob: &SourceBlob) -> ReviewFileContentResponse {
    ReviewFileContentResponse::Binary {
        oid: blob.git_oid.clone(),
        size_bytes: blob.size_bytes,
    }
}

fn nonrenderable_blob(blob: &SourceBlob) -> bool {
    blob.size_bytes > MAX_RENDERED_TEXT_BYTES as u64
}
