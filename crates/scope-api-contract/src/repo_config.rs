use scope_domain::repo_config as domain;
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[cfg_attr(feature = "ts", derive(schemars::JsonSchema, ts_rs::TS))]
#[serde(rename_all = "lowercase")]
pub enum ConfigVisibility {
    Public,
    Private,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[cfg_attr(feature = "ts", derive(schemars::JsonSchema, ts_rs::TS))]
pub struct RepoConfig {
    #[serde(rename = "$schema", default, skip_serializing_if = "Option::is_none")]
    pub schema: Option<String>,
    pub kind: String,
    pub version: u64,
    pub visibility: RepoConfigVisibility,
    #[serde(default)]
    pub history: RepoConfigHistory,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[cfg_attr(feature = "ts", derive(schemars::JsonSchema, ts_rs::TS))]
pub struct RepoConfigVisibility {
    #[serde(default = "default_private_visibility")]
    pub default: ConfigVisibility,
    #[serde(default)]
    pub rules: Vec<RepoConfigVisibilityRule>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[cfg_attr(feature = "ts", derive(schemars::JsonSchema, ts_rs::TS))]
pub struct RepoConfigVisibilityRule {
    pub path: String,
    pub visibility: ConfigVisibility,
}

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[cfg_attr(feature = "ts", derive(schemars::JsonSchema, ts_rs::TS))]
pub struct RepoConfigHistory {
    #[serde(default)]
    pub rewrites: Vec<HistoryRewriteRequest>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[cfg_attr(feature = "ts", derive(schemars::JsonSchema, ts_rs::TS))]
pub struct HistoryRewriteRequest {
    pub path: String,
    pub action: HistoryRewriteAction,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[cfg_attr(feature = "ts", derive(schemars::JsonSchema, ts_rs::TS))]
#[serde(rename_all = "kebab-case")]
pub enum HistoryRewriteAction {
    RedactPublicHistory,
}

fn default_private_visibility() -> ConfigVisibility {
    ConfigVisibility::Private
}

impl From<domain::ConfigVisibility> for ConfigVisibility {
    fn from(value: domain::ConfigVisibility) -> Self {
        match value {
            domain::ConfigVisibility::Public => Self::Public,
            domain::ConfigVisibility::Private => Self::Private,
        }
    }
}

impl From<ConfigVisibility> for domain::ConfigVisibility {
    fn from(value: ConfigVisibility) -> Self {
        match value {
            ConfigVisibility::Public => Self::Public,
            ConfigVisibility::Private => Self::Private,
        }
    }
}

impl From<domain::HistoryRewriteAction> for HistoryRewriteAction {
    fn from(value: domain::HistoryRewriteAction) -> Self {
        match value {
            domain::HistoryRewriteAction::RedactPublicHistory => Self::RedactPublicHistory,
        }
    }
}

impl From<HistoryRewriteAction> for domain::HistoryRewriteAction {
    fn from(value: HistoryRewriteAction) -> Self {
        match value {
            HistoryRewriteAction::RedactPublicHistory => Self::RedactPublicHistory,
        }
    }
}

impl From<domain::RepoConfig> for RepoConfig {
    fn from(value: domain::RepoConfig) -> Self {
        Self {
            schema: value.schema,
            kind: value.kind,
            version: value.version,
            visibility: value.visibility.into(),
            history: value.history.into(),
        }
    }
}

impl From<RepoConfig> for domain::RepoConfig {
    fn from(value: RepoConfig) -> Self {
        Self {
            schema: value.schema,
            kind: value.kind,
            version: value.version,
            visibility: value.visibility.into(),
            history: value.history.into(),
        }
    }
}

impl From<domain::RepoConfigVisibility> for RepoConfigVisibility {
    fn from(value: domain::RepoConfigVisibility) -> Self {
        Self {
            default: value.default.into(),
            rules: value.rules.into_iter().map(Into::into).collect(),
        }
    }
}

impl From<RepoConfigVisibility> for domain::RepoConfigVisibility {
    fn from(value: RepoConfigVisibility) -> Self {
        Self {
            default: value.default.into(),
            rules: value.rules.into_iter().map(Into::into).collect(),
        }
    }
}

impl From<domain::RepoConfigVisibilityRule> for RepoConfigVisibilityRule {
    fn from(value: domain::RepoConfigVisibilityRule) -> Self {
        Self {
            path: value.path,
            visibility: value.visibility.into(),
        }
    }
}

impl From<RepoConfigVisibilityRule> for domain::RepoConfigVisibilityRule {
    fn from(value: RepoConfigVisibilityRule) -> Self {
        Self {
            path: value.path,
            visibility: value.visibility.into(),
        }
    }
}

impl From<domain::RepoConfigHistory> for RepoConfigHistory {
    fn from(value: domain::RepoConfigHistory) -> Self {
        Self {
            rewrites: value.rewrites.into_iter().map(Into::into).collect(),
        }
    }
}

impl From<RepoConfigHistory> for domain::RepoConfigHistory {
    fn from(value: RepoConfigHistory) -> Self {
        Self {
            rewrites: value.rewrites.into_iter().map(Into::into).collect(),
        }
    }
}

impl From<domain::HistoryRewriteRequest> for HistoryRewriteRequest {
    fn from(value: domain::HistoryRewriteRequest) -> Self {
        Self {
            path: value.path,
            action: value.action.into(),
        }
    }
}

impl From<HistoryRewriteRequest> for domain::HistoryRewriteRequest {
    fn from(value: HistoryRewriteRequest) -> Self {
        Self {
            path: value.path,
            action: value.action.into(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn runtime_wire_config_is_json_identical_to_domain_config() {
        let domain = domain::RepoConfig {
            schema: Some("https://scope.dev/repo.schema.json".to_string()),
            kind: "scope.repo".to_string(),
            version: 1,
            visibility: domain::RepoConfigVisibility {
                default: domain::ConfigVisibility::Private,
                rules: vec![domain::RepoConfigVisibilityRule {
                    path: "/README.md".to_string(),
                    visibility: domain::ConfigVisibility::Public,
                }],
            },
            history: domain::RepoConfigHistory {
                rewrites: vec![domain::HistoryRewriteRequest {
                    path: "/secrets/**".to_string(),
                    action: domain::HistoryRewriteAction::RedactPublicHistory,
                }],
            },
        };
        let domain_json = serde_json::to_value(&domain).unwrap();
        let wire = RepoConfig::from(domain.clone());

        assert_eq!(serde_json::to_value(&wire).unwrap(), domain_json);
        assert_eq!(domain::RepoConfig::from(wire), domain);
    }
}
