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
            || path.contains('\\')
            || path.chars().any(char::is_control)
            || path
                .split('/')
                .any(|component| component.is_empty() || component == "." || component == "..")
        {
            return Err(GitTreePathError::Unrepresentable { path });
        }
        if path.split('/').any(is_dot_git_filesystem_alias) {
            return Err(GitTreePathError::ReservedDotGit { path });
        }
        if path.trim_end() != path {
            return Err(GitTreePathError::Unrepresentable { path });
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

fn is_dot_git_filesystem_alias(component: &str) -> bool {
    is_ntfs_dot_git(component) || is_hfs_dot_git(component)
}

fn is_ntfs_dot_git(component: &str) -> bool {
    let component = component.to_ascii_lowercase();
    let suffix = component
        .strip_prefix(".git")
        .or_else(|| component.strip_prefix("git~1"));
    let Some(suffix) = suffix else {
        return false;
    };
    let suffix = suffix.trim_start_matches([' ', '.']);
    suffix.is_empty() || suffix.starts_with(':')
}

fn is_hfs_dot_git(component: &str) -> bool {
    component
        .chars()
        .filter(|character| !is_hfs_ignored(*character))
        .collect::<String>()
        .eq_ignore_ascii_case(".git")
}

fn is_hfs_ignored(character: char) -> bool {
    matches!(
        character,
        '\u{200c}'
            | '\u{200d}'
            | '\u{200e}'
            | '\u{200f}'
            | '\u{202a}'
            | '\u{202b}'
            | '\u{202c}'
            | '\u{202d}'
            | '\u{202e}'
            | '\u{206a}'
            | '\u{206b}'
            | '\u{206c}'
            | '\u{206d}'
            | '\u{206e}'
            | '\u{206f}'
            | '\u{feff}'
    )
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
            " leading-space.txt",
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
    fn rejects_dot_git_and_cross_platform_filesystem_aliases_at_any_depth() {
        for path in [
            ".git",
            ".GiT/config",
            "vendor/.GIT/index",
            "vendor/.git./config",
            "vendor/.git . . /config",
            "vendor/git~1/config",
            "vendor/.git::$INDEX_ALLOCATION/config",
            "vendor/.g\u{200c}it/config",
        ] {
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
