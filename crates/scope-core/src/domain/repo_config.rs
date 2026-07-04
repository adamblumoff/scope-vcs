use super::policy::{ScopePath, Visibility};
use serde::{Deserialize, Serialize};
use thiserror::Error;

pub const REPO_CONFIG_PATH: &str = "/.scope/repo.json";
pub const REPO_CONFIG_KIND: &str = "scope.repo-config";
pub const REPO_CONFIG_VERSION: u64 = 1;

#[derive(Debug, Error)]
pub enum RepoConfigError {
    #[error("repo config is missing")]
    Missing,
    #[error("repo config JSON is invalid: {0}")]
    InvalidJson(serde_json::Error),
    #[error("repo config kind must be scope.repo-config")]
    InvalidKind,
    #[error("repo config version must be 1")]
    InvalidVersion,
    #[error("repo config path must be absolute and start with /")]
    RelativePath,
    #[error("repo config path cannot contain empty segments, . or ..")]
    InvalidSegment,
    #[error("repo config path {0} cannot be made public")]
    ReservedPathPublic(String),
    #[error("repo config rewrite action {0} is unsupported")]
    UnsupportedRewriteAction(String),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ConfigVisibility {
    #[serde(alias = "Public")]
    Public,
    #[serde(alias = "Private")]
    Private,
}

impl From<ConfigVisibility> for Visibility {
    fn from(value: ConfigVisibility) -> Self {
        match value {
            ConfigVisibility::Public => Self::Public,
            ConfigVisibility::Private => Self::Private,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RepoConfig {
    #[serde(rename = "$schema", default, skip_serializing_if = "Option::is_none")]
    pub schema: Option<String>,
    pub kind: String,
    pub version: u64,
    pub visibility: RepoConfigVisibility,
    #[serde(default)]
    pub history: RepoConfigHistory,
}

impl RepoConfig {
    pub fn parse_json(bytes: &[u8]) -> Result<Self, RepoConfigError> {
        let config: Self = serde_json::from_slice(bytes).map_err(RepoConfigError::InvalidJson)?;
        config.validate()?;
        Ok(config)
    }

    pub fn validate(&self) -> Result<(), RepoConfigError> {
        if self.kind != REPO_CONFIG_KIND {
            return Err(RepoConfigError::InvalidKind);
        }
        if self.version != REPO_CONFIG_VERSION {
            return Err(RepoConfigError::InvalidVersion);
        }
        validate_config_path(REPO_CONFIG_PATH)?;
        for rule in &self.visibility.rules {
            validate_config_pattern(&rule.path)?;
            if rule.visibility == ConfigVisibility::Public
                && pattern_matches_path(&rule.path, REPO_CONFIG_PATH)
            {
                return Err(RepoConfigError::ReservedPathPublic(rule.path.clone()));
            }
        }
        for rewrite in &self.history.rewrites {
            validate_config_pattern(&rewrite.path)?;
            if rewrite.action != HistoryRewriteAction::RedactPublicHistory {
                return Err(RepoConfigError::UnsupportedRewriteAction(
                    rewrite.action.as_str().to_string(),
                ));
            }
        }
        Ok(())
    }

    pub fn visibility_for_path(&self, path: &ScopePath) -> Visibility {
        if is_reserved_config_path(path) {
            return Visibility::Private;
        }

        let mut selected = (
            0usize,
            Visibility::from(self.visibility.default_visibility()),
        );
        for rule in &self.visibility.rules {
            if pattern_matches_path(&rule.path, path.as_str()) {
                let weight = pattern_weight(&rule.path);
                if weight >= selected.0 {
                    selected = (weight, Visibility::from(rule.visibility));
                }
            }
        }
        selected.1
    }

    pub fn history_rewrites_added_since(
        &self,
        previous: Option<&RepoConfig>,
    ) -> Vec<HistoryRewriteRequest> {
        self.history
            .rewrites
            .iter()
            .filter(|rewrite| {
                previous.is_none_or(|previous| !previous.history.rewrites.contains(rewrite))
            })
            .cloned()
            .collect()
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RepoConfigVisibility {
    #[serde(default = "default_private_visibility")]
    pub default: ConfigVisibility,
    #[serde(default)]
    pub rules: Vec<RepoConfigVisibilityRule>,
}

impl RepoConfigVisibility {
    pub fn default_visibility(&self) -> ConfigVisibility {
        self.default
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RepoConfigVisibilityRule {
    pub path: String,
    pub visibility: ConfigVisibility,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RepoConfigHistory {
    #[serde(default)]
    pub rewrites: Vec<HistoryRewriteRequest>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct HistoryRewriteRequest {
    pub path: String,
    pub action: HistoryRewriteAction,
}

impl HistoryRewriteRequest {
    pub fn matches_path(&self, path: &ScopePath) -> bool {
        pattern_matches_path(&self.path, path.as_str())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum HistoryRewriteAction {
    RedactPublicHistory,
}

impl HistoryRewriteAction {
    fn as_str(self) -> &'static str {
        match self {
            Self::RedactPublicHistory => "redact-public-history",
        }
    }
}

pub fn is_reserved_config_path(path: &ScopePath) -> bool {
    path.as_str() == REPO_CONFIG_PATH || path.as_str().starts_with("/.scope/")
}

pub fn validate_config_path(path: &str) -> Result<ScopePath, RepoConfigError> {
    let parsed = ScopePath::parse(path).map_err(|error| match error {
        super::policy::PolicyError::RelativePath => RepoConfigError::RelativePath,
        super::policy::PolicyError::InvalidSegment => RepoConfigError::InvalidSegment,
        super::policy::PolicyError::PublicIsland { .. } => RepoConfigError::InvalidSegment,
    })?;
    if parsed.as_str() != path {
        return Err(RepoConfigError::InvalidSegment);
    }
    Ok(parsed)
}

fn validate_config_pattern(pattern: &str) -> Result<(), RepoConfigError> {
    if let Some(base) = pattern.strip_suffix("/**") {
        validate_config_path(base)?;
        return Ok(());
    }
    validate_config_path(pattern)?;
    Ok(())
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

fn default_private_visibility() -> ConfigVisibility {
    ConfigVisibility::Private
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_default_and_rules_determine_visibility() {
        let config = RepoConfig::parse_json(
            br#"{
                "kind": "scope.repo-config",
                "version": 1,
                "visibility": {
                    "default": "private",
                    "rules": [
                        { "path": "/README.md", "visibility": "public" },
                        { "path": "/src/**", "visibility": "public" },
                        { "path": "/src/secrets/**", "visibility": "private" }
                    ]
                }
            }"#,
        )
        .unwrap();

        assert_eq!(
            config.visibility_for_path(&ScopePath::parse("/README.md").unwrap()),
            Visibility::Public
        );
        assert_eq!(
            config.visibility_for_path(&ScopePath::parse("/src/lib.rs").unwrap()),
            Visibility::Public
        );
        assert_eq!(
            config.visibility_for_path(&ScopePath::parse("/src/secrets/key.txt").unwrap()),
            Visibility::Private
        );
        assert_eq!(
            config.visibility_for_path(&ScopePath::parse("/notes.txt").unwrap()),
            Visibility::Private
        );
    }

    #[test]
    fn scope_config_path_is_always_private() {
        let config = RepoConfig::parse_json(
            br#"{
                "kind": "scope.repo-config",
                "version": 1,
                "visibility": {
                    "default": "public",
                    "rules": []
                }
            }"#,
        )
        .unwrap();

        assert_eq!(
            config.visibility_for_path(&ScopePath::parse("/.scope/repo.json").unwrap()),
            Visibility::Private
        );
    }

    #[test]
    fn public_scope_config_rule_is_rejected() {
        let error = RepoConfig::parse_json(
            br#"{
                "kind": "scope.repo-config",
                "version": 1,
                "visibility": {
                    "default": "private",
                    "rules": [
                        { "path": "/.scope/**", "visibility": "public" }
                    ]
                }
            }"#,
        )
        .unwrap_err();

        assert!(matches!(error, RepoConfigError::ReservedPathPublic(_)));
    }

    #[test]
    fn non_canonical_rule_paths_are_rejected() {
        for path in ["/secrets//**", "/secrets/** ", "/README.md "] {
            let config = format!(
                r#"{{
                    "kind": "scope.repo-config",
                    "version": 1,
                    "visibility": {{
                        "default": "public",
                        "rules": [
                            {{ "path": "{path}", "visibility": "private" }}
                        ]
                    }}
                }}"#
            );

            let error = RepoConfig::parse_json(config.as_bytes()).unwrap_err();

            assert!(
                matches!(
                    error,
                    RepoConfigError::InvalidSegment | RepoConfigError::RelativePath
                ),
                "{path} should be rejected, got {error:?}"
            );
        }
    }

    #[test]
    fn non_canonical_rewrite_paths_are_rejected() {
        for path in ["/secrets//**", "/secrets/** ", "/README.md "] {
            let config = format!(
                r#"{{
                    "kind": "scope.repo-config",
                    "version": 1,
                    "visibility": {{
                        "default": "public",
                        "rules": []
                    }},
                    "history": {{
                        "rewrites": [
                            {{ "path": "{path}", "action": "redact-public-history" }}
                        ]
                    }}
                }}"#
            );

            let error = RepoConfig::parse_json(config.as_bytes()).unwrap_err();

            assert!(
                matches!(
                    error,
                    RepoConfigError::InvalidSegment | RepoConfigError::RelativePath
                ),
                "{path} should be rejected, got {error:?}"
            );
        }
    }

    #[test]
    fn history_rewrites_are_accepted_for_supported_actions() {
        let config = RepoConfig::parse_json(
            br#"{
                "kind": "scope.repo-config",
                "version": 1,
                "visibility": {
                    "default": "private",
                    "rules": []
                },
                "history": {
                    "rewrites": [
                        {
                            "path": "/secret.md",
                            "action": "redact-public-history"
                        }
                    ]
                }
            }"#,
        )
        .unwrap();

        let rewrite = &config.history.rewrites[0];
        assert_eq!(rewrite.action, HistoryRewriteAction::RedactPublicHistory);
        assert!(rewrite.matches_path(&ScopePath::parse("/secret.md").unwrap()));
        assert!(!rewrite.matches_path(&ScopePath::parse("/public.md").unwrap()));
    }
}
