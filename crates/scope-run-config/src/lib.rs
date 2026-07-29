use scope_domain::runs::workflow::{
    CompiledWorkflow, ContainerSpec, RunnerSelector, WorkflowError, WorkflowIdentity, WorkflowPath,
    WorkflowRevision, WorkflowStep, WorkflowTriggers,
};
use serde::Deserialize;
use std::collections::BTreeSet;
use thiserror::Error;

pub const MAX_WORKFLOW_DEFINITION_BYTES: usize = 64 * 1024;

#[derive(Debug, Error)]
pub enum RunConfigError {
    #[error("workflow definition exceeds {MAX_WORKFLOW_DEFINITION_BYTES} bytes")]
    DefinitionTooLarge,
    #[error("workflow definition must be UTF-8: {0}")]
    InvalidUtf8(#[from] std::str::Utf8Error),
    #[error("workflow YAML is invalid: {0}")]
    InvalidYaml(#[from] yaml_serde::Error),
    #[error("workflow push trigger supports only branches: [main]")]
    UnsupportedPushBranches,
    #[error("workflow timeout must be an integer followed by s, m, or h")]
    InvalidTimeout,
    #[error("workflow name {0:?} is defined by more than one file")]
    DuplicateWorkflowName(String),
    #[error(transparent)]
    InvalidWorkflow(#[from] WorkflowError),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ParsedWorkflow {
    path: WorkflowPath,
    definition: CompiledWorkflow,
}

impl ParsedWorkflow {
    pub fn path(&self) -> &WorkflowPath {
        &self.path
    }

    pub fn definition(&self) -> &CompiledWorkflow {
        &self.definition
    }

    pub fn into_revision(
        self,
        repository_id: impl Into<String>,
    ) -> Result<WorkflowRevision, WorkflowError> {
        let workflow = WorkflowIdentity::new(repository_id, self.path)?;
        WorkflowRevision::new(workflow, self.definition)
    }
}

pub fn parse_workflow(path: &str, bytes: &[u8]) -> Result<ParsedWorkflow, RunConfigError> {
    if bytes.len() > MAX_WORKFLOW_DEFINITION_BYTES {
        return Err(RunConfigError::DefinitionTooLarge);
    }
    let path = WorkflowPath::parse(path)?;
    let raw: RawWorkflow = yaml_serde::from_str(std::str::from_utf8(bytes)?)?;
    let manual = raw.on.manual;
    let push_main = match raw.on.push {
        None | Some(RawPushTrigger::Enabled(false)) => false,
        Some(RawPushTrigger::Enabled(true)) => true,
        Some(RawPushTrigger::Branches(push)) if push.branches.as_slice() == ["main"] => true,
        Some(RawPushTrigger::Branches(_)) => {
            return Err(RunConfigError::UnsupportedPushBranches);
        }
    };
    let triggers = WorkflowTriggers::new(manual, push_main)?;
    let runner = if raw.runs_on == "any" {
        RunnerSelector::Any
    } else {
        RunnerSelector::named(raw.runs_on)?
    };
    let container = ContainerSpec::new(raw.container.image)?;
    let timeout_seconds = parse_timeout_seconds(&raw.timeout)?;
    let steps = raw
        .steps
        .into_iter()
        .map(|step| WorkflowStep::new(step.name, step.run))
        .collect::<Result<Vec<_>, _>>()?;
    let definition = CompiledWorkflow::new(
        raw.name,
        triggers,
        runner,
        container,
        timeout_seconds,
        steps,
    )?;
    Ok(ParsedWorkflow { path, definition })
}

pub fn parse_workflow_set<'a>(
    repository_id: &str,
    workflows: impl IntoIterator<Item = (&'a str, &'a [u8])>,
) -> Result<Vec<WorkflowRevision>, RunConfigError> {
    let mut parsed = workflows
        .into_iter()
        .map(|(path, bytes)| parse_workflow(path, bytes))
        .collect::<Result<Vec<_>, _>>()?;
    parsed.sort_by(|left, right| left.path().cmp(right.path()));

    let mut names = BTreeSet::new();
    let mut revisions = Vec::with_capacity(parsed.len());
    for workflow in parsed {
        let name = workflow.path().name();
        if !names.insert(name.to_string()) {
            return Err(RunConfigError::DuplicateWorkflowName(name.to_string()));
        }
        revisions.push(workflow.into_revision(repository_id)?);
    }
    Ok(revisions)
}

fn parse_timeout_seconds(timeout: &str) -> Result<u64, RunConfigError> {
    let timeout = timeout.trim();
    let (number, multiplier) = match timeout.as_bytes().last() {
        Some(b's') => (&timeout[..timeout.len() - 1], 1_u64),
        Some(b'm') => (&timeout[..timeout.len() - 1], 60),
        Some(b'h') => (&timeout[..timeout.len() - 1], 60 * 60),
        _ => return Err(RunConfigError::InvalidTimeout),
    };
    let value = number
        .parse::<u64>()
        .map_err(|_| RunConfigError::InvalidTimeout)?;
    value
        .checked_mul(multiplier)
        .ok_or(RunConfigError::InvalidTimeout)
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawWorkflow {
    name: String,
    #[serde(rename = "on")]
    on: RawTriggers,
    #[serde(rename = "runs-on")]
    runs_on: String,
    container: RawContainer,
    timeout: String,
    steps: Vec<RawStep>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawTriggers {
    #[serde(default)]
    manual: bool,
    #[serde(default)]
    push: Option<RawPushTrigger>,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum RawPushTrigger {
    Enabled(bool),
    Branches(RawPushBranches),
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawPushBranches {
    branches: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawContainer {
    image: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawStep {
    name: String,
    run: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    const WORKFLOW: &str = r#"
name: Test
on:
  manual: true
  push:
    branches:
      - main
runs-on: any
container:
  image: rust:1.90
timeout: 20m
steps:
  - name: Format
    run: cargo fmt --check
  - name: Test
    run: cargo test --workspace
"#;

    #[test]
    fn parses_and_normalizes_v1_workflow() {
        let parsed = parse_workflow("/.scope/runs/test.yml", WORKFLOW.as_bytes()).unwrap();
        let definition = parsed.definition();

        assert_eq!(parsed.path().name(), "test");
        assert_eq!(definition.name(), "Test");
        assert!(definition.triggers().manual());
        assert!(definition.triggers().push_main());
        assert_eq!(definition.timeout_seconds(), 20 * 60);
        assert_eq!(definition.container().image(), "rust:1.90");
        assert_eq!(definition.steps()[1].run(), "cargo test --workspace");
    }

    #[test]
    fn equivalent_yaml_has_the_same_revision_digest() {
        let compact = r#"
name: Test
on: { push: true, manual: true }
runs-on: any
container: { image: "rust:1.90" }
timeout: 1200s
steps:
  - { name: Format, run: "cargo fmt --check" }
  - { name: Test, run: "cargo test --workspace" }
"#;
        let first = parse_workflow("/.scope/runs/test.yml", WORKFLOW.as_bytes())
            .unwrap()
            .into_revision("repo-1")
            .unwrap();
        let second = parse_workflow("/.scope/runs/test.yaml", compact.as_bytes())
            .unwrap()
            .into_revision("repo-1")
            .unwrap();

        assert_eq!(first.digest(), second.digest());
    }

    #[test]
    fn rejects_unknown_keys_and_non_main_pushes() {
        let unknown = WORKFLOW.replace("name: Test", "name: Test\nmatrix: {}");
        assert!(matches!(
            parse_workflow("/.scope/runs/test.yml", unknown.as_bytes()),
            Err(RunConfigError::InvalidYaml(_))
        ));

        let branch = WORKFLOW.replace("- main", "- feature");
        assert!(matches!(
            parse_workflow("/.scope/runs/test.yml", branch.as_bytes()),
            Err(RunConfigError::UnsupportedPushBranches)
        ));
    }

    #[test]
    fn rejects_missing_triggers_invalid_timeout_and_oversized_definitions() {
        let no_triggers = WORKFLOW
            .replace("manual: true", "manual: false")
            .replace("push:\n    branches:\n      - main", "push: false");
        assert!(matches!(
            parse_workflow("/.scope/runs/test.yml", no_triggers.as_bytes()),
            Err(RunConfigError::InvalidWorkflow(
                WorkflowError::MissingTrigger
            ))
        ));

        let invalid_timeout = WORKFLOW.replace("timeout: 20m", "timeout: soon");
        assert!(matches!(
            parse_workflow("/.scope/runs/test.yml", invalid_timeout.as_bytes()),
            Err(RunConfigError::InvalidTimeout)
        ));

        let oversized = vec![b'a'; MAX_WORKFLOW_DEFINITION_BYTES + 1];
        assert!(matches!(
            parse_workflow("/.scope/runs/test.yml", &oversized),
            Err(RunConfigError::DefinitionTooLarge)
        ));
    }

    #[test]
    fn workflow_set_rejects_ambiguous_path_stems() {
        let error = parse_workflow_set(
            "repo-1",
            [
                ("/.scope/runs/test.yml", WORKFLOW.as_bytes()),
                ("/.scope/runs/test.yaml", WORKFLOW.as_bytes()),
            ],
        )
        .unwrap_err();

        assert!(matches!(
            error,
            RunConfigError::DuplicateWorkflowName(name) if name == "test"
        ));
    }
}
