use super::{new_client_discussion_id, text::terminal_text};

#[test]
fn terminal_text_replaces_control_characters() {
    assert_eq!(terminal_text("ok\u{1b}[31m\nnext\u{7}"), "ok [31m next ");
}

#[test]
fn client_discussion_ids_are_opaque_and_unique() {
    let first = new_client_discussion_id().unwrap();
    let second = new_client_discussion_id().unwrap();

    assert!(first.starts_with("client_discussion_"));
    assert!(second.starts_with("client_discussion_"));
    assert_ne!(first, second);
}
