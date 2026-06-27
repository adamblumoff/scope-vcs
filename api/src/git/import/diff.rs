use crate::{
    domain::store::{LineDiff, SourceBlob},
    error::ApiError,
    object_store::{ObjectStore, source_blob_text},
};
use similar::{ChangeTag, TextDiff};

const MAX_EXACT_LINE_DIFF_BYTES: u64 = 1024 * 1024;
const MAX_EXACT_LINE_DIFF_LINES: usize = 20_000;

pub(super) fn staged_file_line_diff(
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
