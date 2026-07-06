use super::*;

fn config(default: ConfigVisibility, rules: Vec<(&str, ConfigVisibility)>) -> RepoConfig {
    let rules_json = rules
        .into_iter()
        .map(|(path, visibility)| {
            format!(
                r#"{{ "path": "{path}", "visibility": "{}" }}"#,
                config_visibility_label(visibility)
            )
        })
        .collect::<Vec<_>>()
        .join(", ");
    RepoConfig::parse_json(
        format!(
            r#"{{
  "kind": "scope.repo-config",
  "version": 1,
  "visibility": {{
    "default": "{}",
    "rules": [{rules_json}]
  }},
  "history": {{ "rewrites": [] }}
}}"#,
            config_visibility_label(default)
        )
        .as_bytes(),
    )
    .unwrap()
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
fn folder_toggle_emits_subtree_rule_and_removes_redundant_child_rules() {
    let mut config = config(
        ConfigVisibility::Private,
        vec![("/src/private/**", ConfigVisibility::Public)],
    );

    let result = toggle_visibility_target(
        &mut config,
        directory("/src", vec!["/src/lib.rs", "/src/private/key.txt"]),
    );

    assert!(result.changed);
    assert_eq!(
        config.visibility.rules,
        vec![RepoConfigVisibilityRule {
            path: "/src/**".to_string(),
            visibility: ConfigVisibility::Public,
        }]
    );
}

#[test]
fn mixed_folder_toggle_makes_subtree_public() {
    let mut config = config(
        ConfigVisibility::Private,
        vec![("/src/lib.rs", ConfigVisibility::Public)],
    );
    let target = directory("/src", vec!["/src/lib.rs", "/src/secret.rs"]);

    assert_eq!(target_visibility(&config, &target), ReviewVisibility::Mixed);
    toggle_visibility_target(&mut config, target);

    assert_eq!(
        config.visibility.rules,
        vec![RepoConfigVisibilityRule {
            path: "/src/**".to_string(),
            visibility: ConfigVisibility::Public,
        }]
    );
}

#[test]
fn folder_toggle_private_clears_public_descendant_overrides() {
    let mut config = config(
        ConfigVisibility::Private,
        vec![
            ("/src/a.rs", ConfigVisibility::Public),
            ("/src/b.rs", ConfigVisibility::Public),
        ],
    );
    let target = directory("/src", vec!["/src/a.rs", "/src/b.rs"]);

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
fn folder_toggle_public_preserves_private_descendant_overrides() {
    let mut config = config(
        ConfigVisibility::Private,
        vec![
            ("/src/lib.rs", ConfigVisibility::Public),
            ("/src/secrets/**", ConfigVisibility::Private),
        ],
    );
    let target = directory("/src", vec!["/src/lib.rs", "/src/secrets/key.txt"]);

    assert_eq!(target_visibility(&config, &target), ReviewVisibility::Mixed);
    toggle_visibility_target(&mut config, target);

    assert_eq!(
        config.visibility.rules,
        vec![
            RepoConfigVisibilityRule {
                path: "/src/**".to_string(),
                visibility: ConfigVisibility::Public,
            },
            RepoConfigVisibilityRule {
                path: "/src/secrets/**".to_string(),
                visibility: ConfigVisibility::Private,
            },
        ]
    );
}

#[test]
fn mixed_folder_with_public_base_toggles_private() {
    let mut config = config(
        ConfigVisibility::Private,
        vec![
            ("/src/**", ConfigVisibility::Public),
            ("/src/secrets/**", ConfigVisibility::Private),
        ],
    );
    let target = directory("/src", vec!["/src/lib.rs", "/src/secrets/key.txt"]);

    assert_eq!(target_visibility(&config, &target), ReviewVisibility::Mixed);
    toggle_visibility_target(&mut config, target.clone());

    assert!(config.visibility.rules.is_empty());
    assert_eq!(
        target_visibility(&config, &target),
        ReviewVisibility::Private
    );
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
fn canonicalization_preserves_exact_rule_visibility_against_same_base_subtree() {
    let mut config = config(
        ConfigVisibility::Private,
        vec![
            ("/docs/**", ConfigVisibility::Public),
            ("/docs", ConfigVisibility::Private),
        ],
    );

    canonicalize_visibility_rules(&mut config);

    assert_eq!(
        effective_config_visibility_for_path(&config, "/docs"),
        ConfigVisibility::Private
    );
    assert_eq!(
        config.visibility.rules,
        vec![
            RepoConfigVisibilityRule {
                path: "/docs/**".to_string(),
                visibility: ConfigVisibility::Public,
            },
            RepoConfigVisibilityRule {
                path: "/docs".to_string(),
                visibility: ConfigVisibility::Private,
            },
        ]
    );
}

#[test]
fn canonicalization_preserves_private_subtree_with_same_base_exact_rule() {
    let mut config = config(
        ConfigVisibility::Public,
        vec![
            ("/docs/**", ConfigVisibility::Private),
            ("/docs", ConfigVisibility::Private),
        ],
    );

    canonicalize_visibility_rules(&mut config);

    assert_eq!(
        effective_config_visibility_for_path(&config, "/docs/secret.txt"),
        ConfigVisibility::Private
    );
    assert_eq!(
        config.visibility.rules,
        vec![RepoConfigVisibilityRule {
            path: "/docs/**".to_string(),
            visibility: ConfigVisibility::Private,
        },]
    );
}

#[test]
fn canonicalization_preserves_subtree_winning_same_base_conflict() {
    let mut config = config(
        ConfigVisibility::Public,
        vec![
            ("/docs", ConfigVisibility::Public),
            ("/docs/**", ConfigVisibility::Private),
        ],
    );

    canonicalize_visibility_rules(&mut config);

    assert_eq!(
        effective_config_visibility_for_path(&config, "/docs"),
        ConfigVisibility::Private
    );
    assert_eq!(
        effective_config_visibility_for_path(&config, "/docs/secret.txt"),
        ConfigVisibility::Private
    );
    assert_eq!(
        config.visibility.rules,
        vec![
            RepoConfigVisibilityRule {
                path: "/docs".to_string(),
                visibility: ConfigVisibility::Public,
            },
            RepoConfigVisibilityRule {
                path: "/docs/**".to_string(),
                visibility: ConfigVisibility::Private,
            },
        ]
    );
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
