use serde::{Deserialize, Serialize};
use std::fmt;
use thiserror::Error;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum PolicyError {
    #[error("path must be absolute and start with /")]
    RelativePath,
    #[error("path cannot contain empty segments, . or ..")]
    InvalidSegment,
    #[error("public rule at {child} cannot live under private parent {parent}")]
    PublicIsland { child: ScopePath, parent: ScopePath },
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct ScopePath(String);

impl ScopePath {
    pub fn parse(input: impl AsRef<str>) -> Result<Self, PolicyError> {
        let raw = input.as_ref().trim();
        if !raw.starts_with('/') {
            return Err(PolicyError::RelativePath);
        }

        let mut parts = Vec::new();
        for part in raw.split('/') {
            if part.is_empty() {
                continue;
            }
            if part == "." || part == ".." {
                return Err(PolicyError::InvalidSegment);
            }
            parts.push(part);
        }

        if parts.is_empty() {
            Ok(Self("/".to_string()))
        } else {
            Ok(Self(format!("/{}", parts.join("/"))))
        }
    }

    pub fn root() -> Self {
        Self("/".to_string())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn is_ancestor_of(&self, other: &ScopePath) -> bool {
        self.0 == "/"
            || other.0 == self.0
            || other
                .0
                .strip_prefix(self.0.as_str())
                .is_some_and(|suffix| suffix.starts_with('/'))
    }
}

impl fmt::Display for ScopePath {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum PrincipalKind {
    User,
    Team,
    Org,
    Agent,
    Ci,
    Public,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Principal {
    pub id: String,
    pub kind: PrincipalKind,
}

impl Principal {
    pub fn public() -> Self {
        Self {
            id: "public".to_string(),
            kind: PrincipalKind::Public,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(any(test, feature = "ts"), derive(ts_rs::TS))]
pub enum Visibility {
    Public,
    Private,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct VisibilityRule {
    pub path: ScopePath,
    pub visibility: Visibility,
}

impl VisibilityRule {
    pub fn public(path: ScopePath) -> Self {
        Self {
            path,
            visibility: Visibility::Public,
        }
    }

    pub fn private(path: ScopePath) -> Self {
        Self {
            path,
            visibility: Visibility::Private,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Policy {
    default_visibility: Visibility,
    rules: Vec<VisibilityRule>,
}

impl Policy {
    pub fn new(default_visibility: Visibility) -> Self {
        Self {
            default_visibility,
            rules: Vec::new(),
        }
    }

    pub fn add_rule(&mut self, rule: VisibilityRule) -> Result<(), PolicyError> {
        if rule.visibility == Visibility::Public
            && let Some(parent) = self.private_ancestor_for(&rule.path)
        {
            return Err(PolicyError::PublicIsland {
                child: rule.path,
                parent,
            });
        }

        self.rules.retain(|existing| existing.path != rule.path);
        self.rules.push(rule);
        self.rules
            .sort_by(|left, right| left.path.as_str().cmp(right.path.as_str()));
        self.validate_no_public_islands()?;
        Ok(())
    }

    pub fn effective_rule(&self, path: &ScopePath) -> Option<&VisibilityRule> {
        self.rules
            .iter()
            .filter(|rule| rule.path.is_ancestor_of(path))
            .max_by_key(|rule| rule.path.as_str().len())
    }

    pub fn effective_visibility(&self, path: &ScopePath) -> Visibility {
        self.effective_rule(path)
            .map(|rule| rule.visibility)
            .unwrap_or(self.default_visibility)
    }

    pub fn set_default_visibility(&mut self, visibility: Visibility) {
        self.default_visibility = visibility;
    }

    pub fn can_read(&self, path: &ScopePath, can_read_private_files: bool) -> bool {
        match self.effective_visibility(path) {
            Visibility::Public => true,
            Visibility::Private => can_read_private_files,
        }
    }

    pub fn rules(&self) -> &[VisibilityRule] {
        &self.rules
    }

    fn private_ancestor_for(&self, path: &ScopePath) -> Option<ScopePath> {
        self.rules
            .iter()
            .find(|rule| {
                rule.visibility == Visibility::Private
                    && rule.path != *path
                    && rule.path.is_ancestor_of(path)
            })
            .map(|rule| rule.path.clone())
    }

    fn validate_no_public_islands(&self) -> Result<(), PolicyError> {
        for rule in self
            .rules
            .iter()
            .filter(|rule| rule.visibility == Visibility::Public)
        {
            if let Some(parent) = self.private_ancestor_for(&rule.path) {
                return Err(PolicyError::PublicIsland {
                    child: rule.path.clone(),
                    parent,
                });
            }
        }
        Ok(())
    }
}
