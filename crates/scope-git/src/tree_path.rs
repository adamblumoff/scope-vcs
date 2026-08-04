use scope_domain::policy::ScopePath;
use std::fmt;
use thiserror::Error;

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct GitTreePath(String);

impl GitTreePath {
    pub fn parse(path: impl Into<String>) -> Result<Self, GitTreePathError> {
        let path = path.into();
        if path.is_empty()
            || path.starts_with('/')
            || path.trim() != path
            || path.contains('\\')
            || path.chars().any(char::is_control)
            || path
                .split('/')
                .any(|component| component.is_empty() || component == "." || component == "..")
        {
            return Err(GitTreePathError::Unrepresentable { path });
        }
        if path
            .split('/')
            .any(|component| component.eq_ignore_ascii_case(".git"))
        {
            return Err(GitTreePathError::ReservedDotGit { path });
        }
        Ok(Self(path))
    }

    pub fn from_scope_path(path: &ScopePath) -> Result<Self, GitTreePathError> {
        let relative = path
            .as_str()
            .strip_prefix('/')
            .expect("Scope paths are absolute");
        Self::parse(relative)
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn to_scope_path(&self) -> ScopePath {
        ScopePath::parse(format!("/{}", self.0))
            .expect("Git tree paths are canonical Scope file paths")
    }
}

impl fmt::Display for GitTreePath {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

#[derive(Clone, Debug, Error, PartialEq, Eq)]
pub enum GitTreePathError {
    #[error("Git tree path {path:?} cannot be represented safely")]
    Unrepresentable { path: String },
    #[error("Git tree path {path:?} contains the reserved .git component")]
    ReservedDotGit { path: String },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_canonical_repository_relative_file_paths() {
        for path in [
            "README.md",
            "docs/read me.md",
            ".scope/RULES.md",
            "src/café.rs",
        ] {
            assert_eq!(GitTreePath::parse(path).unwrap().as_str(), path);
        }
    }

    #[test]
    fn rejects_paths_that_cannot_be_materialized_consistently() {
        for path in [
            "",
            "/README.md",
            "README.md ",
            "dir\\file.txt",
            "line\nbreak.txt",
            "./README.md",
            "docs/../README.md",
            "docs//README.md",
            "docs/",
        ] {
            assert!(matches!(
                GitTreePath::parse(path),
                Err(GitTreePathError::Unrepresentable { .. })
            ));
        }
    }

    #[test]
    fn rejects_dot_git_at_any_depth_and_ascii_case() {
        for path in [".git", ".GiT/config", "vendor/.GIT/index"] {
            assert!(matches!(
                GitTreePath::parse(path),
                Err(GitTreePathError::ReservedDotGit { .. })
            ));
        }
    }

    #[test]
    fn converts_canonical_scope_paths_without_changing_identity() {
        let scope_path = ScopePath::parse("/docs/guide.md").unwrap();
        let git_path = GitTreePath::from_scope_path(&scope_path).unwrap();

        assert_eq!(git_path.as_str(), "docs/guide.md");
        assert_eq!(git_path.to_scope_path(), scope_path);
    }
}
