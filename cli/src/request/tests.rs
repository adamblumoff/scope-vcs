use super::{RequestBaseAudience, parse_base_audience_config, text::terminal_text};

#[test]
fn terminal_text_replaces_control_characters() {
    assert_eq!(terminal_text("ok\u{1b}[31m\nnext\u{7}"), "ok [31m next ");
}

#[test]
fn base_audience_config_round_trips() {
    assert_eq!(
        parse_base_audience_config("public").unwrap(),
        RequestBaseAudience::Public
    );
    assert_eq!(
        parse_base_audience_config("private").unwrap(),
        RequestBaseAudience::Private
    );
    assert!(parse_base_audience_config("member").is_err());
}
