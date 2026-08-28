pub fn repository_key(path: &str) -> Option<String> {
    let mut segments = path.strip_prefix("/git/")?.split('/');
    match segments.next()? {
        "public" | "permissioned" => {}
        _ => return None,
    }
    let owner = non_empty_segment(segments.next()?)?;
    let repository = non_empty_segment(segments.next()?)?;
    Some(format!("{owner}/{repository}"))
}

fn non_empty_segment(value: &str) -> Option<&str> {
    (!value.is_empty()).then_some(value)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_the_same_key_from_each_git_operation() {
        for path in [
            "/git/public/scope/router/info/refs",
            "/git/public/scope/router/git-upload-pack",
            "/git/permissioned/scope/router/info/refs",
            "/git/permissioned/scope/router/git-receive-pack",
        ] {
            assert_eq!(repository_key(path).as_deref(), Some("scope/router"));
        }
    }

    #[test]
    fn preserves_canonical_percent_encoded_segments() {
        assert_eq!(
            repository_key("/git/permissioned/an%20owner/a%2Frepo/info/refs").as_deref(),
            Some("an%20owner/a%2Frepo")
        );
    }

    #[test]
    fn rejects_non_git_and_unknown_mode_paths() {
        assert_eq!(repository_key("/healthz"), None);
        assert_eq!(repository_key("/git/private/scope/router/info/refs"), None);
        assert_eq!(repository_key("/git/public//router/info/refs"), None);
    }
}
