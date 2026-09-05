use serde::{Deserialize, Serialize};
use std::{borrow::Borrow, collections::BTreeMap, fmt};
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

impl Borrow<str> for ScopePath {
    fn borrow(&self) -> &str {
        self.as_str()
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
        self.add_rules([rule])
    }

    pub fn add_rules(
        &mut self,
        rules: impl IntoIterator<Item = VisibilityRule>,
    ) -> Result<(), PolicyError> {
        let mut additions = rules.into_iter().peekable();
        if additions.peek().is_none() {
            return Ok(());
        }
        let rules = self
            .rules
            .iter()
            .cloned()
            .chain(additions)
            .map(|rule| (rule.path, rule.visibility))
            .collect::<BTreeMap<_, _>>();
        for (path, visibility) in &rules {
            if *visibility != Visibility::Public {
                continue;
            }
            // Check proper path ancestors, including root, rather than every rule.
            for (separator, _) in path.as_str().match_indices('/') {
                let ancestor = if separator == 0 {
                    "/"
                } else {
                    &path.as_str()[..separator]
                };
                if ancestor != path.as_str() && rules.get(ancestor) == Some(&Visibility::Private) {
                    return Err(PolicyError::PublicIsland {
                        child: path.clone(),
                        parent: ScopePath(ancestor.to_string()),
                    });
                }
            }
        }
        self.rules = rules
            .into_iter()
            .map(|(path, visibility)| VisibilityRule { path, visibility })
            .collect();
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

    pub fn remove_rule(&mut self, path: &ScopePath) {
        self.rules.retain(|rule| &rule.path != path);
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
}
