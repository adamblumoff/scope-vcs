use super::review_body_heights;

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
