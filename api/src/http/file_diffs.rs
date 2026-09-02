use crate::{
    error::ApiError,
    git::content::source_content_bytes,
    http::responses::{ReviewFileContentResponse, ReviewFileDiffResponse},
    state::AppState,
};
use scope_domain::history::FileChangeKind;
use scope_domain::{
    content::SourceBlob,
    repository::RepositoryIncarnation,
    repository::git::{GitHead, GitPackSpan},
};

pub(crate) const MAX_RENDERED_TEXT_BYTES: usize = 1024 * 1024;

pub(crate) async fn review_file_diff_response_for_blobs(
    state: &AppState,
    git_source: Option<(RepositoryIncarnation, &GitHead, &[GitPackSpan])>,
    path: String,
    kind: FileChangeKind,
    old_content: Option<&SourceBlob>,
    new_content: Option<&SourceBlob>,
) -> Result<ReviewFileDiffResponse, ApiError> {
    let old_mode = old_content.map(|blob| blob.git_file_mode.clone());
    let new_mode = new_content.map(|blob| blob.git_file_mode.clone());
    let old_response = match old_content {
        Some(blob) => {
            Some(review_content_response_for_blob(state, blob, git_source.clone()).await?)
        }
        None => None,
    };
    let new_response = match new_content {
        Some(blob) => Some(review_content_response_for_blob(state, blob, git_source).await?),
        None => None,
    };
    Ok(ReviewFileDiffResponse {
        path,
        kind: kind.into(),
        old_mode,
        new_mode,
        old_content: old_response,
        new_content: new_response,
    })
}

pub(crate) async fn review_content_response_for_blob(
    state: &AppState,
    blob: &SourceBlob,
    git_source: Option<(RepositoryIncarnation, &GitHead, &[GitPackSpan])>,
) -> Result<ReviewFileContentResponse, ApiError> {
    if nonrenderable_blob(blob) {
        return Ok(binary_content(blob));
    }

    let bytes = source_content_bytes(state, blob, git_source).await?;
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
