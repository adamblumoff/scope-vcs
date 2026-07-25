use super::{actions::ready_stake, new_client_discussion_id, text::terminal_text};
use scope_core::domain::requests::REQUEST_MAX_STAKE_CREDITS;

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

#[test]
fn ready_stake_is_required_only_when_the_request_uses_credits() {
    let missing = ready_stake(true, None).unwrap_err().to_string();
    assert!(missing.contains("--stake <CREDITS>"));
    assert!(missing.contains(&format!("1–{REQUEST_MAX_STAKE_CREDITS}")));

    assert_eq!(ready_stake(true, Some(12)).unwrap(), Some(12));
    assert_eq!(ready_stake(false, None).unwrap(), None);
    assert_eq!(ready_stake(false, Some(12)).unwrap(), None);
}
