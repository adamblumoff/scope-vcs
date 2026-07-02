use crate::{
    domain::store::{SourceBlob, StagedFileChangeKind},
    error::ApiError,
    http::responses::{ReviewFileContentResponse, ReviewFileDiffResponse, ReviewLineDiffResponse},
    object_store::{ObjectStore, source_blob_bytes},
};
use similar::{ChangeTag, TextDiff};

const MAX_RENDERED_TEXT_BYTES: usize = 1024 * 1024;
const MAX_DIFF_TEXT_LINES: usize = 20_000;

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

pub(crate) fn review_line_diff_for_blobs(
    store: &dyn ObjectStore,
    old_content: Option<&SourceBlob>,
    new_content: Option<&SourceBlob>,
) -> Result<ReviewLineDiffResponse, ApiError> {
    if old_content.is_some_and(nonrenderable_blob) || new_content.is_some_and(nonrenderable_blob) {
        return Ok(ReviewLineDiffResponse::default());
    }

    let old_bytes = old_content
        .map(|blob| source_blob_bytes(store, blob))
        .transpose()?;
    let new_bytes = new_content
        .map(|blob| source_blob_bytes(store, blob))
        .transpose()?;
    Ok(review_line_diff_from_bytes(
        old_bytes.as_deref(),
        new_bytes.as_deref(),
    ))
}

pub(crate) fn add_line_diff(target: &mut ReviewLineDiffResponse, next: ReviewLineDiffResponse) {
    target.additions += next.additions;
    target.deletions += next.deletions;
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

fn review_line_diff_from_bytes(
    old_content: Option<&[u8]>,
    new_content: Option<&[u8]>,
) -> ReviewLineDiffResponse {
    let Some(old_content) = renderable_text(old_content.unwrap_or_default()) else {
        return ReviewLineDiffResponse::default();
    };
    let Some(new_content) = renderable_text(new_content.unwrap_or_default()) else {
        return ReviewLineDiffResponse::default();
    };

    line_diff_between(old_content, new_content)
}

fn renderable_text(bytes: &[u8]) -> Option<&str> {
    if bytes.len() > MAX_RENDERED_TEXT_BYTES {
        return None;
    }
    std::str::from_utf8(bytes).ok()
}

fn line_diff_between(old_content: &str, new_content: &str) -> ReviewLineDiffResponse {
    let mut line_diff = ReviewLineDiffResponse::default();
    if exceeds_diff_line_limit(old_content) || exceeds_diff_line_limit(new_content) {
        return line_diff;
    }

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

fn exceeds_diff_line_limit(content: &str) -> bool {
    content.lines().count() > MAX_DIFF_TEXT_LINES
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
    fn binary_content_has_no_line_diff() {
        let diff = review_line_diff_from_bytes(Some(b"hello"), Some(&[0xff, 0x00, 0x61]));

        assert_eq!(diff.deletions, 0);
        assert_eq!(diff.additions, 0);
    }

    #[test]
    fn high_line_count_text_has_no_line_diff() {
        let many_lines = "x\n".repeat(MAX_DIFF_TEXT_LINES + 1);
        let diff = line_diff_between("", &many_lines);

        assert_eq!(diff.deletions, 0);
        assert_eq!(diff.additions, 0);
    }
}
