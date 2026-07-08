pub(super) fn short_oid(oid: &str) -> String {
    oid.chars().take(12).collect()
}

pub(super) fn terminal_text(value: &str) -> String {
    value
        .chars()
        .map(|character| {
            if character.is_control() {
                ' '
            } else {
                character
            }
        })
        .collect()
}
