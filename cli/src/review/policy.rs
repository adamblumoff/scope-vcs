use super::tree::{ReviewNode, ReviewNodeKind, ReviewTree};
use scope_core::domain::{
    policy::{ScopePath, Visibility},
    repo_config::{ConfigVisibility, RepoConfig, RepoConfigVisibilityRule},
};
use std::collections::BTreeMap;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ReviewVisibility {
    Public,
    Private,
    Mixed,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ToggleResult {
    pub changed: bool,
    pub message: String,
}

pub fn toggle_node_visibility(
    config: &mut RepoConfig,
    tree: &ReviewTree,
    node_id: usize,
) -> ToggleResult {
    let node = tree.node(node_id);
    if node.reserved {
        return ToggleResult {
            changed: false,
            message: ".scope files are always private".to_string(),
        };
    }

    let before = config.visibility.rules.clone();
    let before_default = config.visibility.default;
    match node.kind {
        ReviewNodeKind::Root => {
            config.visibility.default = opposite_config_visibility(config.visibility.default);
        }
        ReviewNodeKind::Directory => {
            let next = next_directory_visibility(config, tree, node_id);
            replace_visibility_rules_in_subtree(config, &node.path, next);
            upsert_visibility_rule(config, folder_rule_path(&node.path), next);
        }
        ReviewNodeKind::File => {
            if file_path_collides_with_pattern_syntax(&node.path) {
                return ToggleResult {
                    changed: false,
                    message: format!(
                        "{} cannot be configured with current pattern syntax",
                        node.name
                    ),
                };
            }
            let current = effective_config_visibility_for_path(config, &node.path);
            remove_same_base_folder_rule(config, &node.path);
            upsert_visibility_rule(
                config,
                node.path.clone(),
                opposite_config_visibility(current),
            );
        }
    }
    canonicalize_visibility_rules(config);

    ToggleResult {
        changed: config.visibility.rules != before || config.visibility.default != before_default,
        message: format!(
            "{} set to {}",
            node.name,
            visibility_label(node_visibility(config, tree, node_id))
        ),
    }
}

pub fn node_visibility(config: &RepoConfig, tree: &ReviewTree, node_id: usize) -> ReviewVisibility {
    let node = tree.node(node_id);
    if node.reserved {
        return ReviewVisibility::Private;
    }
    if node.kind == ReviewNodeKind::Root {
        return aggregate_visibility(
            tree.file_paths_under(node_id)
                .iter()
                .map(|path| effective_config_visibility_for_path(config, path)),
            config.visibility.default,
        );
    }
    if node.kind == ReviewNodeKind::File {
        return config_visibility_to_review(effective_config_visibility_for_path(
            config, &node.path,
        ));
    }

    let file_paths = tree.file_paths_under(node_id);
    if file_paths.is_empty() {
        return config_visibility_to_review(effective_config_visibility_for_path(
            config, &node.path,
        ));
    }
    aggregate_visibility(
        file_paths
            .iter()
            .map(|path| effective_config_visibility_for_path(config, path)),
        config.visibility.default,
    )
}

pub fn rule_label(config: &RepoConfig, node: &ReviewNode) -> String {
    if node.reserved {
        return "forced private".to_string();
    }
    if node.kind == ReviewNodeKind::Root {
        return format!(
            "default {}",
            config_visibility_label(config.visibility.default)
        );
    }

    let direct_rule_path = match node.kind {
        ReviewNodeKind::Root => None,
        ReviewNodeKind::Directory => Some(folder_rule_path(&node.path)),
        ReviewNodeKind::File => Some(node.path.clone()),
    };
    if let Some(path) = direct_rule_path
        && config.visibility.rules.iter().any(|rule| rule.path == path)
    {
        return format!("explicit {path}");
    }

    matching_visibility_rule(config, &node.path)
        .map(|rule| format!("inherited {}", rule.path))
        .unwrap_or_else(|| "inherited default".to_string())
}

pub fn visibility_label(visibility: ReviewVisibility) -> &'static str {
    match visibility {
        ReviewVisibility::Public => "public",
        ReviewVisibility::Private => "private",
        ReviewVisibility::Mixed => "mixed",
    }
}

pub fn config_visibility_label(visibility: ConfigVisibility) -> &'static str {
    match visibility {
        ConfigVisibility::Public => "public",
        ConfigVisibility::Private => "private",
    }
}

pub fn canonicalize_visibility_rules(config: &mut RepoConfig) {
    let base_visibilities = effective_visibilities_by_rule_base(config);
    let mut rules_by_path = BTreeMap::new();
    for rule in &config.visibility.rules {
        if rule.visibility == ConfigVisibility::Public && is_reserved_scope_pattern(&rule.path) {
            continue;
        }
        rules_by_path.insert(rule.path.clone(), rule.visibility);
    }
    config.visibility.rules = rules_by_path
        .into_iter()
        .map(|(path, visibility)| RepoConfigVisibilityRule { path, visibility })
        .collect();

    loop {
        let Some(index) = redundant_rule_index(config) else {
            break;
        };
        config.visibility.rules.remove(index);
    }
    restore_rule_base_visibilities(config, base_visibilities.clone());
    sort_visibility_rules(config, &base_visibilities);
}

fn redundant_rule_index(config: &RepoConfig) -> Option<usize> {
    config
        .visibility
        .rules
        .iter()
        .enumerate()
        .find_map(|(index, rule)| {
            let mut without_rule = config.clone();
            without_rule.visibility.rules.remove(index);
            rule_is_redundant(&without_rule, rule).then_some(index)
        })
}

fn rule_is_redundant(config_without_rule: &RepoConfig, rule: &RepoConfigVisibilityRule) -> bool {
    let base = rule_base_path(&rule.path);
    if effective_config_visibility_for_path(config_without_rule, base) != rule.visibility {
        return false;
    }

    if !rule.path.ends_with("/**") {
        return true;
    }

    let descendant_probe = format!("{base}/__scope_probe__");
    effective_config_visibility_for_path(config_without_rule, &descendant_probe) == rule.visibility
}

fn upsert_visibility_rule(config: &mut RepoConfig, path: String, visibility: ConfigVisibility) {
    config.visibility.rules.retain(|rule| rule.path != path);
    config
        .visibility
        .rules
        .push(RepoConfigVisibilityRule { path, visibility });
}

fn effective_visibilities_by_rule_base(config: &RepoConfig) -> BTreeMap<String, ConfigVisibility> {
    config
        .visibility
        .rules
        .iter()
        .map(|rule| {
            let base = rule_base_path(&rule.path).to_string();
            let visibility = effective_config_visibility_for_path(config, &base);
            (base, visibility)
        })
        .collect()
}

fn restore_rule_base_visibilities(
    config: &mut RepoConfig,
    base_visibilities: BTreeMap<String, ConfigVisibility>,
) {
    for (base, visibility) in base_visibilities {
        if effective_config_visibility_for_path(config, &base) != visibility {
            upsert_visibility_rule(config, base, visibility);
        }
    }
}

fn sort_visibility_rules(
    config: &mut RepoConfig,
    base_visibilities: &BTreeMap<String, ConfigVisibility>,
) {
    config.visibility.rules.sort_by(|left, right| {
        rule_base_path(&left.path)
            .cmp(rule_base_path(&right.path))
            .then_with(|| {
                semantic_sort_rank(left, base_visibilities)
                    .cmp(&semantic_sort_rank(right, base_visibilities))
            })
            .then_with(|| rule_sort_rank(&left.path).cmp(&rule_sort_rank(&right.path)))
            .then_with(|| left.path.cmp(&right.path))
    });
}

fn semantic_sort_rank(
    rule: &RepoConfigVisibilityRule,
    base_visibilities: &BTreeMap<String, ConfigVisibility>,
) -> u8 {
    let base = rule_base_path(&rule.path);
    if base_visibilities.get(base).copied() == Some(rule.visibility) {
        1
    } else {
        0
    }
}

fn rule_sort_rank(path: &str) -> u8 {
    if path.ends_with("/**") { 0 } else { 1 }
}

fn replace_visibility_rules_in_subtree(
    config: &mut RepoConfig,
    folder_path: &str,
    next: ConfigVisibility,
) {
    config.visibility.rules.retain(|rule| {
        if !pattern_is_inside_subtree(&rule.path, folder_path) {
            return true;
        }

        next == ConfigVisibility::Public && rule.visibility == ConfigVisibility::Private
    });
}

fn remove_same_base_folder_rule(config: &mut RepoConfig, file_path: &str) {
    let stale_folder_rule = folder_rule_path(file_path);
    config
        .visibility
        .rules
        .retain(|rule| rule.path != stale_folder_rule);
}

fn aggregate_visibility(
    visibilities: impl Iterator<Item = ConfigVisibility>,
    fallback: ConfigVisibility,
) -> ReviewVisibility {
    let mut selected = None;
    for visibility in visibilities {
        match selected {
            None => selected = Some(visibility),
            Some(previous) if previous == visibility => {}
            Some(_) => return ReviewVisibility::Mixed,
        }
    }
    config_visibility_to_review(selected.unwrap_or(fallback))
}

fn effective_config_visibility_for_path(config: &RepoConfig, path: &str) -> ConfigVisibility {
    let Ok(scope_path) = ScopePath::parse(path) else {
        return config.visibility.default;
    };
    match config.visibility_for_path(&scope_path) {
        Visibility::Public => ConfigVisibility::Public,
        Visibility::Private => ConfigVisibility::Private,
    }
}

fn next_directory_visibility(
    config: &RepoConfig,
    tree: &ReviewTree,
    node_id: usize,
) -> ConfigVisibility {
    let node = tree.node(node_id);
    match node_visibility(config, tree, node_id) {
        ReviewVisibility::Public => ConfigVisibility::Private,
        ReviewVisibility::Private => ConfigVisibility::Public,
        ReviewVisibility::Mixed => {
            opposite_config_visibility(effective_config_visibility_for_path(config, &node.path))
        }
    }
}

fn opposite_config_visibility(visibility: ConfigVisibility) -> ConfigVisibility {
    match visibility {
        ConfigVisibility::Public => ConfigVisibility::Private,
        ConfigVisibility::Private => ConfigVisibility::Public,
    }
}

fn config_visibility_to_review(visibility: ConfigVisibility) -> ReviewVisibility {
    match visibility {
        ConfigVisibility::Public => ReviewVisibility::Public,
        ConfigVisibility::Private => ReviewVisibility::Private,
    }
}

fn matching_visibility_rule<'a>(
    config: &'a RepoConfig,
    path: &str,
) -> Option<&'a RepoConfigVisibilityRule> {
    config
        .visibility
        .rules
        .iter()
        .filter(|rule| pattern_matches_path(&rule.path, path))
        .max_by_key(|rule| pattern_weight(&rule.path))
}

fn folder_rule_path(path: &str) -> String {
    format!("{path}/**")
}

fn pattern_matches_path(pattern: &str, path: &str) -> bool {
    if let Some(base) = pattern.strip_suffix("/**") {
        return path == base
            || path
                .strip_prefix(base)
                .is_some_and(|tail| tail.starts_with('/'));
    }
    path == pattern
}

fn pattern_weight(pattern: &str) -> usize {
    pattern.strip_suffix("/**").unwrap_or(pattern).len()
}

fn rule_base_path(pattern: &str) -> &str {
    pattern.strip_suffix("/**").unwrap_or(pattern)
}

fn is_reserved_scope_pattern(pattern: &str) -> bool {
    let base = rule_base_path(pattern);
    base == "/.scope" || base.starts_with("/.scope/")
}

fn pattern_is_inside_subtree(pattern: &str, folder_path: &str) -> bool {
    let base = pattern.strip_suffix("/**").unwrap_or(pattern);
    base == folder_path
        || base
            .strip_prefix(folder_path)
            .is_some_and(|tail| tail.starts_with('/'))
}

fn file_path_collides_with_pattern_syntax(path: &str) -> bool {
    path.ends_with("/**")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::review::tree::ReviewTree;

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

    #[test]
    fn folder_toggle_emits_subtree_rule_and_removes_redundant_child_rules() {
        let tree = ReviewTree::from_paths(
            &[
                "src/lib.rs".to_string(),
                "src/private/key.txt".to_string(),
                "README.md".to_string(),
            ],
            &[],
        );
        let src_id = tree
            .nodes()
            .iter()
            .find(|node| node.path == "/src")
            .unwrap()
            .id;
        let mut config = config(
            ConfigVisibility::Private,
            vec![("/src/private/**", ConfigVisibility::Public)],
        );

        let result = toggle_node_visibility(&mut config, &tree, src_id);

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
        let tree = ReviewTree::from_paths(
            &["src/lib.rs".to_string(), "src/secret.rs".to_string()],
            &[],
        );
        let src_id = tree
            .nodes()
            .iter()
            .find(|node| node.path == "/src")
            .unwrap()
            .id;
        let mut config = config(
            ConfigVisibility::Private,
            vec![("/src/lib.rs", ConfigVisibility::Public)],
        );

        assert_eq!(
            node_visibility(&config, &tree, src_id),
            ReviewVisibility::Mixed
        );
        toggle_node_visibility(&mut config, &tree, src_id);

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
        let tree = ReviewTree::from_paths(&["src/a.rs".to_string(), "src/b.rs".to_string()], &[]);
        let src_id = tree
            .nodes()
            .iter()
            .find(|node| node.path == "/src")
            .unwrap()
            .id;
        let mut config = config(
            ConfigVisibility::Private,
            vec![
                ("/src/a.rs", ConfigVisibility::Public),
                ("/src/b.rs", ConfigVisibility::Public),
            ],
        );

        assert_eq!(
            node_visibility(&config, &tree, src_id),
            ReviewVisibility::Public
        );
        toggle_node_visibility(&mut config, &tree, src_id);

        assert!(config.visibility.rules.is_empty());
        assert_eq!(
            node_visibility(&config, &tree, src_id),
            ReviewVisibility::Private
        );
    }

    #[test]
    fn folder_toggle_public_preserves_private_descendant_overrides() {
        let tree = ReviewTree::from_paths(
            &["src/lib.rs".to_string(), "src/secrets/key.txt".to_string()],
            &[],
        );
        let src_id = tree
            .nodes()
            .iter()
            .find(|node| node.path == "/src")
            .unwrap()
            .id;
        let mut config = config(
            ConfigVisibility::Private,
            vec![
                ("/src/lib.rs", ConfigVisibility::Public),
                ("/src/secrets/**", ConfigVisibility::Private),
            ],
        );

        assert_eq!(
            node_visibility(&config, &tree, src_id),
            ReviewVisibility::Mixed
        );
        toggle_node_visibility(&mut config, &tree, src_id);

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
        let tree = ReviewTree::from_paths(
            &["src/lib.rs".to_string(), "src/secrets/key.txt".to_string()],
            &[],
        );
        let src_id = tree
            .nodes()
            .iter()
            .find(|node| node.path == "/src")
            .unwrap()
            .id;
        let mut config = config(
            ConfigVisibility::Private,
            vec![
                ("/src/**", ConfigVisibility::Public),
                ("/src/secrets/**", ConfigVisibility::Private),
            ],
        );

        assert_eq!(
            node_visibility(&config, &tree, src_id),
            ReviewVisibility::Mixed
        );
        toggle_node_visibility(&mut config, &tree, src_id);

        assert!(config.visibility.rules.is_empty());
        assert_eq!(
            node_visibility(&config, &tree, src_id),
            ReviewVisibility::Private
        );
    }

    #[test]
    fn file_toggle_removes_stale_same_base_folder_rule() {
        let tree = ReviewTree::from_paths(&["docs".to_string()], &[]);
        let docs_id = tree
            .nodes()
            .iter()
            .find(|node| node.path == "/docs")
            .unwrap()
            .id;
        let mut config = config(
            ConfigVisibility::Private,
            vec![("/docs/**", ConfigVisibility::Public)],
        );

        assert_eq!(
            node_visibility(&config, &tree, docs_id),
            ReviewVisibility::Public
        );
        toggle_node_visibility(&mut config, &tree, docs_id);

        assert!(config.visibility.rules.is_empty());
        assert_eq!(
            node_visibility(&config, &tree, docs_id),
            ReviewVisibility::Private
        );
    }

    #[test]
    fn file_toggle_refuses_paths_that_collide_with_subtree_pattern_syntax() {
        let tree = ReviewTree::from_paths(&["src/**".to_string()], &[]);
        let file_id = tree
            .nodes()
            .iter()
            .find(|node| node.path == "/src/**")
            .unwrap()
            .id;
        let mut config = config(ConfigVisibility::Private, vec![]);

        let result = toggle_node_visibility(&mut config, &tree, file_id);

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
        let tree = ReviewTree::from_paths(&[".scope/repo.json".to_string()], &[]);
        let config_id = tree
            .nodes()
            .iter()
            .find(|node| node.path == "/.scope/repo.json")
            .unwrap()
            .id;
        let mut config = config(ConfigVisibility::Public, vec![]);

        let result = toggle_node_visibility(&mut config, &tree, config_id);

        assert!(!result.changed);
        assert!(config.visibility.rules.is_empty());
        assert_eq!(
            node_visibility(&config, &tree, config_id),
            ReviewVisibility::Private
        );
    }
}
