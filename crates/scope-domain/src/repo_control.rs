use crate::{policy::ScopePath, runs::workflow::WorkflowPath};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RepoControlPath {
    Workflow(WorkflowPath),
    Forbidden,
}

pub fn classify_repo_control_path(path: &ScopePath) -> Option<RepoControlPath> {
    if !is_private_control_path(path) {
        return None;
    }
    Some(
        WorkflowPath::parse(path.as_str())
            .map(RepoControlPath::Workflow)
            .unwrap_or(RepoControlPath::Forbidden),
    )
}

pub fn is_private_control_path(path: &ScopePath) -> bool {
    path.as_str() == "/.scope" || path.as_str().starts_with("/.scope/")
}

pub fn is_private_control_pattern(pattern: &str) -> bool {
    let base = pattern.strip_suffix("/**").unwrap_or(pattern);
    base == "/.scope" || base.starts_with("/.scope/")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn path(value: &str) -> ScopePath {
        ScopePath::parse(value).unwrap()
    }

    #[test]
    fn classifier_separates_workflows_from_forbidden_control_paths() {
        assert_eq!(classify_repo_control_path(&path("/README.md")), None);
        assert!(matches!(
            classify_repo_control_path(&path("/.scope/runs/test.yml")),
            Some(RepoControlPath::Workflow(workflow)) if workflow.name() == "test"
        ));
        for forbidden in [
            "/.scope",
            "/.scope/repo.json",
            "/.scope/runs",
            "/.scope/runs/Test.yml",
            "/.scope/runs/nested/test.yml",
            "/.scope/other.yml",
        ] {
            assert_eq!(
                classify_repo_control_path(&path(forbidden)),
                Some(RepoControlPath::Forbidden),
                "{forbidden}"
            );
        }
    }

    #[test]
    fn scope_control_patterns_are_private() {
        for pattern in ["/.scope", "/.scope/**", "/.scope/runs/test.yml"] {
            assert!(is_private_control_pattern(pattern), "{pattern}");
        }
        for pattern in ["/README.md", "/src/**", "/.scope-notes/**"] {
            assert!(!is_private_control_pattern(pattern), "{pattern}");
        }
    }
}
