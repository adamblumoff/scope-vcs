use super::*;

fn config(default: ConfigVisibility, rules: Vec<(&str, ConfigVisibility)>) -> RepoConfig {
    let mut config = RepoConfig::with_default_visibility(default);
    config.visibility.rules = rules
        .into_iter()
        .map(|(path, visibility)| RepoConfigVisibilityRule {
            path: path.to_string(),
            visibility,
        })
        .collect();
    config.validate().unwrap();
    config
}

fn directory<'a>(path: &'a str, file_paths_under: Vec<&'a str>) -> VisibilityTarget<'a> {
    VisibilityTarget {
        name: path.rsplit('/').next().unwrap_or(path),
        path,
        kind: VisibilityNodeKind::Directory,
        reserved: false,
        file_paths_under,
    }
}

fn file(path: &str) -> VisibilityTarget<'_> {
    VisibilityTarget {
        name: path.rsplit('/').next().unwrap_or(path),
        path,
        kind: VisibilityNodeKind::File,
        reserved: false,
        file_paths_under: vec![path],
    }
}

#[test]
fn folder_toggle_canonicalizes_subtree_rules() {
    let public = ConfigVisibility::Public;
    let private = ConfigVisibility::Private;
    let cases = [
        (
            vec![("/src/private/**", public)],
            vec!["/src/lib.rs", "/src/private/key"],
            vec![("/src/**", public)],
        ),
        (
            vec![("/src/lib.rs", public)],
            vec!["/src/lib.rs", "/src/secret.rs"],
            vec![("/src/**", public)],
        ),
        (
            vec![("/src/a.rs", public), ("/src/b.rs", public)],
            vec!["/src/a.rs", "/src/b.rs"],
            vec![],
        ),
        (
            vec![("/src/lib.rs", public), ("/src/secrets/**", private)],
            vec!["/src/lib.rs", "/src/secrets/key"],
            vec![("/src/**", public), ("/src/secrets/**", private)],
        ),
        (
            vec![("/src/**", public), ("/src/secrets/**", private)],
            vec!["/src/lib.rs", "/src/secrets/key"],
            vec![],
        ),
    ];
    for (rules, paths, expected) in cases {
        let mut config = config(private, rules);
        assert!(toggle_visibility_target(&mut config, directory("/src", paths)).changed);
        assert_eq!(
            config.visibility.rules,
            expected
                .into_iter()
                .map(|(path, visibility)| RepoConfigVisibilityRule {
                    path: path.to_string(),
                    visibility,
                })
                .collect::<Vec<_>>()
        );
    }
}

#[test]
fn file_toggle_removes_stale_same_base_folder_rule() {
    let mut config = config(
        ConfigVisibility::Private,
        vec![("/docs/**", ConfigVisibility::Public)],
    );
    let target = file("/docs");

    assert_eq!(
        target_visibility(&config, &target),
        ReviewVisibility::Public
    );
    toggle_visibility_target(&mut config, target.clone());

    assert!(config.visibility.rules.is_empty());
    assert_eq!(
        target_visibility(&config, &target),
        ReviewVisibility::Private
    );
}

#[test]
fn file_toggle_refuses_paths_that_collide_with_subtree_pattern_syntax() {
    let mut config = config(ConfigVisibility::Private, vec![]);

    let result = toggle_visibility_target(&mut config, file("/src/**"));

    assert!(!result.changed);
    assert!(result.message.contains("pattern syntax"));
    assert!(config.visibility.rules.is_empty());
}

#[test]
fn canonicalization_preserves_same_base_exact_and_subtree_semantics() {
    let public = ConfigVisibility::Public;
    let private = ConfigVisibility::Private;
    let cases = [
        (
            private,
            vec![("/docs/**", public), ("/docs", private)],
            vec![("/docs/**", public), ("/docs", private)],
        ),
        (
            public,
            vec![("/docs/**", private), ("/docs", private)],
            vec![("/docs/**", private)],
        ),
        (
            public,
            vec![("/docs", public), ("/docs/**", private)],
            vec![("/docs", public), ("/docs/**", private)],
        ),
    ];
    for (default, rules, expected) in cases {
        let mut config = config(default, rules);
        canonicalize_visibility_rules(&mut config);
        assert_eq!(
            effective_config_visibility_for_path(&config, "/docs"),
            private
        );
        assert_eq!(
            config.visibility.rules,
            expected
                .into_iter()
                .map(|(path, visibility)| RepoConfigVisibilityRule {
                    path: path.to_string(),
                    visibility,
                })
                .collect::<Vec<_>>()
        );
    }
}

#[test]
fn reserved_scope_paths_cannot_be_toggled_public() {
    let mut config = config(ConfigVisibility::Public, vec![]);
    let target = VisibilityTarget {
        name: "repo.json",
        path: "/.scope/repo.json",
        kind: VisibilityNodeKind::File,
        reserved: true,
        file_paths_under: vec!["/.scope/repo.json"],
    };

    let result = toggle_visibility_target(&mut config, target.clone());

    assert!(!result.changed);
    assert!(config.visibility.rules.is_empty());
    assert_eq!(
        target_visibility(&config, &target),
        ReviewVisibility::Private
    );
}
