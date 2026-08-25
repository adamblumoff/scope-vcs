pub(super) fn is_kebab_name(name: &str, max_bytes: usize) -> bool {
    !name.is_empty()
        && name.len() <= max_bytes
        && !name.starts_with('-')
        && !name.ends_with('-')
        && !name.contains("--")
        && name
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
}
