use crate::{policy::ScopePath, runs::workflow::WorkflowPath};

pub const REPO_RULES_PATH: &str = "/.scope/RULES.md";

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RepoControlPath {
    Rules,
    Workflow(WorkflowPath),
    Forbidden,
}

pub fn classify_repo_control_path(path: &ScopePath) -> Option<RepoControlPath> {
    if !is_repo_control_path(path) {
        return None;
    }
    if is_repo_rules_path(path) {
        return Some(RepoControlPath::Rules);
    }
    Some(
        WorkflowPath::parse(path.as_str())
            .map(RepoControlPath::Workflow)
            .unwrap_or(RepoControlPath::Forbidden),
    )
}

pub fn is_repo_control_path(path: &ScopePath) -> bool {
    path.as_str() == "/.scope" || path.as_str().starts_with("/.scope/")
}

pub fn is_repo_rules_path(path: &ScopePath) -> bool {
    path.as_str() == REPO_RULES_PATH
}

pub fn is_public_request_protected_path(path: &ScopePath) -> bool {
    is_case_folded_repo_control_path(path) || is_agent_context_path(path)
}

fn is_case_folded_repo_control_path(path: &ScopePath) -> bool {
    path.as_str()
        .trim_start_matches('/')
        .split('/')
        .next()
        .is_some_and(|root| root.eq_ignore_ascii_case(".scope"))
}

fn is_agent_context_path(path: &ScopePath) -> bool {
    let relative = path.as_str().trim_start_matches('/');
    let root = relative.split('/').next().unwrap_or(relative);
    [".codex", ".claude", ".agents"]
        .iter()
        .any(|directory| root.eq_ignore_ascii_case(directory))
        || relative.eq_ignore_ascii_case(".mcp.json")
        || relative.rsplit('/').next().is_some_and(|name| {
            [
                "AGENTS.md",
                "AGENTS.override.md",
                "CLAUDE.md",
                "CLAUDE.local.md",
            ]
            .iter()
            .any(|protected| name.eq_ignore_ascii_case(protected))
        })
}

pub fn is_private_control_path(path: &ScopePath) -> bool {
    is_repo_control_path(path) && !is_repo_rules_path(path)
}

pub fn is_repo_control_pattern(pattern: &str) -> bool {
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
        assert_eq!(
            classify_repo_control_path(&path(REPO_RULES_PATH)),
            Some(RepoControlPath::Rules)
        );
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
    fn rules_are_control_but_not_private_control() {
        let rules = path(REPO_RULES_PATH);
        assert!(is_repo_control_path(&rules));
        assert!(!is_private_control_path(&rules));
    }

    #[test]
    fn public_requests_cannot_change_native_agent_context() {
        for protected in [
            "/AGENTS.md",
            "/src/AGENTS.md",
            "/AGENTS.override.md",
            "/src/AGENTS.override.md",
            "/CLAUDE.md",
            "/docs/CLAUDE.md",
            "/CLAUDE.local.md",
            "/.codex/config.toml",
            "/.claude/settings.json",
            "/.agents/skills/review/SKILL.md",
            "/.mcp.json",
            "/.SCOPE/RULES.md",
            "/agents.md",
            "/src/Agents.Override.Md",
            "/claude.MD",
            "/.CoDeX/config.toml",
            "/.CLAUDE/settings.json",
            "/.AGENTS/skills/review/SKILL.md",
            "/.MCP.JSON",
        ] {
            assert!(
                is_public_request_protected_path(&path(protected)),
                "{protected}"
            );
        }
        for ordinary in ["/README.md", "/src/agent-notes.md", "/notes/CLAUDE.txt"] {
            assert!(
                !is_public_request_protected_path(&path(ordinary)),
                "{ordinary}"
            );
        }
    }

    #[test]
    fn scope_control_patterns_are_reserved() {
        for pattern in ["/.scope", "/.scope/**", "/.scope/runs/test.yml"] {
            assert!(is_repo_control_pattern(pattern), "{pattern}");
        }
        for pattern in ["/README.md", "/src/**", "/.scope-notes/**"] {
            assert!(!is_repo_control_pattern(pattern), "{pattern}");
        }
    }
}
