use crate::{
    domain::store::{FileChangeKind, SourceBlob},
    error::ApiError,
    git::content::source_content_bytes,
    http::responses::{ReviewFileContentResponse, ReviewFileDiffResponse},
    state::AppState,
};

pub(crate) const MAX_RENDERED_TEXT_BYTES: usize = 1024 * 1024;

pub(crate) fn review_file_diff_response_for_blobs(
    state: &AppState,
    path: String,
    kind: FileChangeKind,
    old_content: Option<&SourceBlob>,
    new_content: Option<&SourceBlob>,
) -> Result<ReviewFileDiffResponse, ApiError> {
    Ok(ReviewFileDiffResponse {
        path,
        kind,
        old_mode: old_content.map(|blob| blob.git_file_mode.clone()),
        new_mode: new_content.map(|blob| blob.git_file_mode.clone()),
        old_content: old_content
            .map(|blob| review_content_response_for_blob(state, blob))
            .transpose()?,
        new_content: new_content
            .map(|blob| review_content_response_for_blob(state, blob))
            .transpose()?,
    })
}

pub(crate) fn review_content_response_for_blob(
    state: &AppState,
    blob: &SourceBlob,
) -> Result<ReviewFileContentResponse, ApiError> {
    if nonrenderable_blob(blob) {
        return Ok(binary_content(blob));
    }

    let bytes = source_content_bytes(state, blob)?;
    Ok(review_content_from_bytes(blob, &bytes))
}

fn review_content_from_bytes(blob: &SourceBlob, bytes: &[u8]) -> ReviewFileContentResponse {
    review_content_response_for_bytes(&blob.git_oid, bytes)
}

pub(crate) fn review_content_response_for_bytes(
    oid: &str,
    bytes: &[u8],
) -> ReviewFileContentResponse {
    if bytes.len() <= MAX_RENDERED_TEXT_BYTES
        && let Ok(text) = std::str::from_utf8(bytes)
    {
        return ReviewFileContentResponse::Text {
            text: text.to_string(),
        };
    }

    ReviewFileContentResponse::Binary {
        oid: oid.to_string(),
        size_bytes: bytes.len() as u64,
    }
}

pub(crate) fn binary_content_response(oid: &str, size_bytes: u64) -> ReviewFileContentResponse {
    ReviewFileContentResponse::Binary {
        oid: oid.to_string(),
        size_bytes,
    }
}

fn binary_content(blob: &SourceBlob) -> ReviewFileContentResponse {
    binary_content_response(&blob.git_oid, blob.size_bytes)
}

fn nonrenderable_blob(blob: &SourceBlob) -> bool {
    blob.size_bytes > MAX_RENDERED_TEXT_BYTES as u64
}
