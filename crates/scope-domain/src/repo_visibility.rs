use super::{
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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum VisibilityNodeKind {
    Root,
    Directory,
    File,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ToggleResult {
    pub changed: bool,
    pub message: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct VisibilityTarget<'a> {
    pub name: &'a str,
    pub path: &'a str,
    pub kind: VisibilityNodeKind,
    pub reserved: bool,
    pub file_paths_under: Vec<&'a str>,
}

pub fn toggle_visibility_target(
    config: &mut RepoConfig,
    target: VisibilityTarget<'_>,
) -> ToggleResult {
    if target.reserved {
        return ToggleResult {
            changed: false,
            message: ".scope files are always private".to_string(),
        };
    }

    let before = config.visibility.rules.clone();
    let before_default = config.visibility.default;
    match target.kind {
        VisibilityNodeKind::Root => {
            config.visibility.default = opposite_config_visibility(config.visibility.default);
        }
        VisibilityNodeKind::Directory => {
            let next = next_directory_visibility(config, &target);
            replace_visibility_rules_in_subtree(config, target.path, next);
            upsert_visibility_rule(config, folder_rule_path(target.path), next);
        }
        VisibilityNodeKind::File => {
            if file_path_collides_with_pattern_syntax(target.path) {
                return ToggleResult {
                    changed: false,
                    message: format!(
                        "{} cannot be configured with current pattern syntax",
                        target.name
                    ),
                };
            }
            let current = effective_config_visibility_for_path(config, target.path);
            remove_same_base_folder_rule(config, target.path);
            upsert_visibility_rule(
                config,
                target.path.to_string(),
                opposite_config_visibility(current),
            );
        }
    }
    canonicalize_visibility_rules(config);

    ToggleResult {
        changed: config.visibility.rules != before || config.visibility.default != before_default,
        message: format!(
            "{} set to {}",
            target.name,
            visibility_label(target_visibility(config, &target))
        ),
    }
}

pub fn target_visibility(config: &RepoConfig, target: &VisibilityTarget<'_>) -> ReviewVisibility {
    if target.reserved {
        return ReviewVisibility::Private;
    }
    if target.kind == VisibilityNodeKind::Root {
        return aggregate_visibility(
            target
                .file_paths_under
                .iter()
                .map(|path| effective_config_visibility_for_path(config, path)),
            config.visibility.default,
        );
    }
    if target.kind == VisibilityNodeKind::File {
        return config_visibility_to_review(effective_config_visibility_for_path(
            config,
            target.path,
        ));
    }

    if target.file_paths_under.is_empty() {
        return config_visibility_to_review(effective_config_visibility_for_path(
            config,
            target.path,
        ));
    }
    aggregate_visibility(
        target
            .file_paths_under
            .iter()
            .map(|path| effective_config_visibility_for_path(config, path)),
        config.visibility.default,
    )
}

pub fn rule_label(config: &RepoConfig, target: &VisibilityTarget<'_>) -> String {
    if target.reserved {
        return "forced private".to_string();
    }
    if target.kind == VisibilityNodeKind::Root {
        return format!(
            "default {}",
            config_visibility_label(config.visibility.default)
        );
    }

    let direct_rule_path = match target.kind {
        VisibilityNodeKind::Root => None,
        VisibilityNodeKind::Directory => Some(folder_rule_path(target.path)),
        VisibilityNodeKind::File => Some(target.path.to_string()),
    };
    if let Some(path) = direct_rule_path
        && config.visibility.rules.iter().any(|rule| rule.path == path)
    {
        return format!("explicit {path}");
    }

    matching_visibility_rule(config, target.path)
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
    target: &VisibilityTarget<'_>,
) -> ConfigVisibility {
    match target_visibility(config, target) {
        ReviewVisibility::Public => ConfigVisibility::Private,
        ReviewVisibility::Private => ConfigVisibility::Public,
        ReviewVisibility::Mixed => {
            opposite_config_visibility(effective_config_visibility_for_path(config, target.path))
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
#[path = "repo_visibility_tests.rs"]
mod tests;
