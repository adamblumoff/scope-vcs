use super::{fit_cell, footer_hint, review_body_heights, row_line};
use crate::review::state::{ChangeListKind, ReviewMode, ReviewRow};
use scope_domain::repo_visibility::ReviewVisibility;
use unicode_width::UnicodeWidthStr;

#[test]
fn body_layout_keeps_a_file_row_visible_when_read_only_summaries_overflow() {
    assert_eq!(review_body_heights(3, 10, 5), (2, 1));
    assert_eq!(review_body_heights(1, 10, 5), (0, 1));
}

#[test]
fn body_layout_uses_available_space_when_summaries_fit() {
    assert_eq!(review_body_heights(5, 2, 5), (2, 3));
    assert_eq!(review_body_heights(5, 10, 0), (5, 0));
}

#[test]
fn tree_rows_use_web_visibility_icons_and_stay_within_terminal_width() {
    let row = |visibility| ReviewRow::TreeNode {
        depth: 1,
        name: "a-very-long-file-name-that-needs-to-be-truncated.rs".to_string(),
        path: "/a-very-long-file-name-that-needs-to-be-truncated.rs".to_string(),
        kind: crate::review::tree::ReviewNodeKind::File,
        expanded: false,
        visibility,
        rule: "inherited /some/very/long/folder/**".to_string(),
        reserved: false,
        change_status: Some("A".to_string()),
    };

    let public_line = row_line(&row(ReviewVisibility::Public), false, 80).to_string();
    let private_line = row_line(&row(ReviewVisibility::Private), false, 80).to_string();
    assert!(public_line.contains("🌐 public"), "{public_line}");
    assert!(private_line.contains("🔒 private"), "{private_line}");
    assert!(!public_line.ends_with("  A"), "{public_line}");
    assert_eq!(UnicodeWidthStr::width(public_line.as_str()), 80);
    assert_eq!(UnicodeWidthStr::width(private_line.as_str()), 80);
}

#[test]
fn change_section_rows_are_compact_and_descriptive() {
    let row = ReviewRow::ChangeSection {
        kind: ChangeListKind::Deleted,
        count: 87,
        expanded: false,
    };
    let line = row_line(&row, false, 80).to_string();

    assert!(line.starts_with("[>] Deleted files (87)"), "{line}");
    assert_eq!(UnicodeWidthStr::width(line.as_str()), 80);
}

#[test]
fn narrow_footer_keeps_required_push_actions_visible() {
    let hint = fit_cell(&footer_hint(ReviewMode::Push, 80), 80);

    assert!(hint.contains("P push"), "{hint}");
    assert!(hint.contains("Q cancel"), "{hint}");
    assert!(hint.contains("? help"), "{hint}");
    assert_eq!(UnicodeWidthStr::width(hint.as_str()), 80);
}
