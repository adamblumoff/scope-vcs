use super::runner::RunnerName;
use serde::{Deserialize, Deserializer, Serialize, de::Error as _};
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;
use thiserror::Error;

pub const MAX_WORKFLOW_NAME_BYTES: usize = 100;
pub const MAX_WORKFLOW_PATH_NAME_BYTES: usize = 64;
pub const MAX_CONTAINER_IMAGE_BYTES: usize = 512;
pub const MAX_WORKFLOW_STEPS: usize = 64;
pub const MAX_STEP_NAME_BYTES: usize = 100;
pub const MAX_STEP_COMMAND_BYTES: usize = 64 * 1024;
pub const MAX_WORKFLOW_TIMEOUT_SECONDS: u64 = 24 * 60 * 60;

const WORKFLOW_PATH_PREFIX: &str = "/.scope/runs/";
const WORKFLOW_DIGEST_VERSION: u8 = 1;

#[derive(Debug, Error)]
pub enum WorkflowError {
    #[error("workflow repository id is required")]
    MissingRepositoryId,
    #[error("workflow path must be /.scope/runs/<kebab-name>.yml or .yaml")]
    InvalidPath,
    #[error("workflow name must contain between 1 and {MAX_WORKFLOW_NAME_BYTES} bytes")]
    InvalidName,
    #[error("workflow must enable at least one trigger")]
    MissingTrigger,
    #[error("runner name is invalid")]
    InvalidRunnerName,
    #[error("container image must contain between 1 and {MAX_CONTAINER_IMAGE_BYTES} bytes")]
    InvalidContainerImage,
    #[error("workflow timeout must be between 1 and {MAX_WORKFLOW_TIMEOUT_SECONDS} seconds")]
    InvalidTimeout,
    #[error("workflow must contain at least one step")]
    MissingSteps,
    #[error("workflow cannot contain more than {MAX_WORKFLOW_STEPS} steps")]
    TooManySteps,
    #[error("workflow step name must contain between 1 and {MAX_STEP_NAME_BYTES} bytes")]
    InvalidStepName,
    #[error("workflow step name {0:?} is duplicated")]
    DuplicateStepName(String),
    #[error("workflow step command must contain between 1 and {MAX_STEP_COMMAND_BYTES} bytes")]
    InvalidStepCommand,
    #[error("workflow revision digest could not be serialized: {0}")]
    Digest(serde_json::Error),
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(transparent)]
pub struct WorkflowPath(String);

impl WorkflowPath {
    pub fn parse(path: impl Into<String>) -> Result<Self, WorkflowError> {
        let path = path.into();
        let Some(file_name) = path.strip_prefix(WORKFLOW_PATH_PREFIX) else {
            return Err(WorkflowError::InvalidPath);
        };
        if file_name.contains('/') {
            return Err(WorkflowError::InvalidPath);
        }
        let stem = file_name
            .strip_suffix(".yaml")
            .or_else(|| file_name.strip_suffix(".yml"))
            .ok_or(WorkflowError::InvalidPath)?;
        if !is_kebab_name(stem) {
            return Err(WorkflowError::InvalidPath);
        }
        Ok(Self(path))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn name(&self) -> &str {
        self.0
            .strip_prefix(WORKFLOW_PATH_PREFIX)
            .and_then(|file_name| {
                file_name
                    .strip_suffix(".yaml")
                    .or_else(|| file_name.strip_suffix(".yml"))
            })
            .expect("validated workflow paths always have a supported extension")
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct WorkflowIdentity {
    repository_id: String,
    path: WorkflowPath,
}

impl WorkflowIdentity {
    pub fn new(
        repository_id: impl Into<String>,
        path: WorkflowPath,
    ) -> Result<Self, WorkflowError> {
        let repository_id = repository_id.into();
        if repository_id.trim().is_empty() {
            return Err(WorkflowError::MissingRepositoryId);
        }
        Ok(Self {
            repository_id,
            path,
        })
    }

    pub fn repository_id(&self) -> &str {
        &self.repository_id
    }

    pub fn path(&self) -> &WorkflowPath {
        &self.path
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct WorkflowTriggers {
    manual: bool,
    push_main: bool,
}

impl WorkflowTriggers {
    pub fn new(manual: bool, push_main: bool) -> Result<Self, WorkflowError> {
        if !manual && !push_main {
            return Err(WorkflowError::MissingTrigger);
        }
        Ok(Self { manual, push_main })
    }

    pub fn manual(&self) -> bool {
        self.manual
    }

    pub fn push_main(&self) -> bool {
        self.push_main
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", content = "name", rename_all = "kebab-case")]
pub enum RunnerSelector {
    Any,
    Named(String),
}

impl RunnerSelector {
    pub fn named(name: impl Into<String>) -> Result<Self, WorkflowError> {
        RunnerName::parse(name.into())
            .map(|name| Self::Named(name.as_str().to_string()))
            .map_err(|_| WorkflowError::InvalidRunnerName)
    }

    pub fn matches_name(&self, name: &str) -> bool {
        match self {
            Self::Any => true,
            Self::Named(expected) => expected == name,
        }
    }

    pub(crate) fn validate(&self) -> Result<(), WorkflowError> {
        match self {
            Self::Any => Ok(()),
            Self::Named(name) => RunnerName::parse(name.clone())
                .map(|_| ())
                .map_err(|_| WorkflowError::InvalidRunnerName),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct ContainerSpec {
    image: String,
}

impl ContainerSpec {
    pub fn new(image: impl Into<String>) -> Result<Self, WorkflowError> {
        let image = image.into();
        if image.is_empty()
            || image.len() > MAX_CONTAINER_IMAGE_BYTES
            || image.chars().any(char::is_whitespace)
        {
            return Err(WorkflowError::InvalidContainerImage);
        }
        Ok(Self { image })
    }

    pub fn image(&self) -> &str {
        &self.image
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct WorkflowStep {
    name: String,
    run: String,
}

impl WorkflowStep {
    pub fn new(name: impl Into<String>, run: impl Into<String>) -> Result<Self, WorkflowError> {
        let name = name.into();
        let run = run.into();
        if name.trim().is_empty() || name.len() > MAX_STEP_NAME_BYTES {
            return Err(WorkflowError::InvalidStepName);
        }
        if run.trim().is_empty() || run.len() > MAX_STEP_COMMAND_BYTES {
            return Err(WorkflowError::InvalidStepCommand);
        }
        Ok(Self {
            name: name.trim().to_string(),
            run,
        })
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn run(&self) -> &str {
        &self.run
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct CompiledWorkflow {
    name: String,
    triggers: WorkflowTriggers,
    runner: RunnerSelector,
    container: ContainerSpec,
    timeout_seconds: u64,
    steps: Vec<WorkflowStep>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PersistedCompiledWorkflow {
    name: String,
    triggers: PersistedWorkflowTriggers,
    runner: PersistedRunnerSelector,
    container: PersistedContainerSpec,
    timeout_seconds: u64,
    steps: Vec<PersistedWorkflowStep>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PersistedWorkflowTriggers {
    manual: bool,
    push_main: bool,
}

#[derive(Deserialize)]
#[serde(tag = "kind", content = "name", rename_all = "kebab-case")]
enum PersistedRunnerSelector {
    Any,
    Named(String),
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PersistedContainerSpec {
    image: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PersistedWorkflowStep {
    name: String,
    run: String,
}

impl<'de> Deserialize<'de> for CompiledWorkflow {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let persisted = PersistedCompiledWorkflow::deserialize(deserializer)?;
        let triggers =
            WorkflowTriggers::new(persisted.triggers.manual, persisted.triggers.push_main)
                .map_err(D::Error::custom)?;
        let runner = match persisted.runner {
            PersistedRunnerSelector::Any => RunnerSelector::Any,
            PersistedRunnerSelector::Named(name) => {
                RunnerSelector::named(name).map_err(D::Error::custom)?
            }
        };
        let container = ContainerSpec::new(persisted.container.image).map_err(D::Error::custom)?;
        let steps = persisted
            .steps
            .into_iter()
            .map(|step| WorkflowStep::new(step.name, step.run))
            .collect::<Result<Vec<_>, _>>()
            .map_err(D::Error::custom)?;
        Self::new(
            persisted.name,
            triggers,
            runner,
            container,
            persisted.timeout_seconds,
            steps,
        )
        .map_err(D::Error::custom)
    }
}

impl CompiledWorkflow {
    pub fn new(
        name: impl Into<String>,
        triggers: WorkflowTriggers,
        runner: RunnerSelector,
        container: ContainerSpec,
        timeout_seconds: u64,
        steps: Vec<WorkflowStep>,
    ) -> Result<Self, WorkflowError> {
        let name = name.into();
        let name = name.trim();
        if name.is_empty() || name.len() > MAX_WORKFLOW_NAME_BYTES {
            return Err(WorkflowError::InvalidName);
        }
        if timeout_seconds == 0 || timeout_seconds > MAX_WORKFLOW_TIMEOUT_SECONDS {
            return Err(WorkflowError::InvalidTimeout);
        }
        runner.validate()?;
        if steps.is_empty() {
            return Err(WorkflowError::MissingSteps);
        }
        if steps.len() > MAX_WORKFLOW_STEPS {
            return Err(WorkflowError::TooManySteps);
        }
        let mut step_names = BTreeSet::new();
        for step in &steps {
            if !step_names.insert(step.name.clone()) {
                return Err(WorkflowError::DuplicateStepName(step.name.clone()));
            }
        }
        Ok(Self {
            name: name.to_string(),
            triggers,
            runner,
            container,
            timeout_seconds,
            steps,
        })
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn triggers(&self) -> &WorkflowTriggers {
        &self.triggers
    }

    pub fn runner(&self) -> &RunnerSelector {
        &self.runner
    }

    pub fn container(&self) -> &ContainerSpec {
        &self.container
    }

    pub fn timeout_seconds(&self) -> u64 {
        self.timeout_seconds
    }

    pub fn steps(&self) -> &[WorkflowStep] {
        &self.steps
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct WorkflowRevision {
    workflow: WorkflowIdentity,
    digest: String,
    definition: CompiledWorkflow,
}

impl WorkflowRevision {
    pub fn new(
        workflow: WorkflowIdentity,
        definition: CompiledWorkflow,
    ) -> Result<Self, WorkflowError> {
        #[derive(Serialize)]
        struct DigestInput<'a> {
            version: u8,
            definition: &'a CompiledWorkflow,
        }

        let bytes = serde_json::to_vec(&DigestInput {
            version: WORKFLOW_DIGEST_VERSION,
            definition: &definition,
        })
        .map_err(WorkflowError::Digest)?;
        let digest = hex::encode(Sha256::digest(bytes));
        Ok(Self {
            workflow,
            digest,
            definition,
        })
    }

    pub fn workflow(&self) -> &WorkflowIdentity {
        &self.workflow
    }

    pub fn digest(&self) -> &str {
        &self.digest
    }

    pub fn definition(&self) -> &CompiledWorkflow {
        &self.definition
    }
}

fn is_kebab_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= MAX_WORKFLOW_PATH_NAME_BYTES
        && !name.starts_with('-')
        && !name.ends_with('-')
        && !name.contains("--")
        && name
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
}

#[cfg(test)]
mod tests {
    use super::*;

    fn compiled_workflow() -> CompiledWorkflow {
        CompiledWorkflow::new(
            "Test",
            WorkflowTriggers::new(true, true).unwrap(),
            RunnerSelector::Any,
            ContainerSpec::new("rust:1.90").unwrap(),
            20 * 60,
            vec![
                WorkflowStep::new("Format", "cargo fmt --check").unwrap(),
                WorkflowStep::new("Test", "cargo test --workspace").unwrap(),
            ],
        )
        .unwrap()
    }

    #[test]
    fn workflow_paths_are_exact_and_stable() {
        let path = WorkflowPath::parse("/.scope/runs/test-api.yaml").unwrap();
        assert_eq!(path.name(), "test-api");
        for invalid in [
            ".scope/runs/test.yml",
            "/.scope/runs/Test.yml",
            "/.scope/runs/test.json",
            "/.scope/runs/nested/test.yml",
            "/.scope/runs/-test.yml",
            "/.scope/runs/test--api.yml",
            "/.scope/runs/test_.yml",
        ] {
            assert!(
                matches!(
                    WorkflowPath::parse(invalid),
                    Err(WorkflowError::InvalidPath)
                ),
                "{invalid} should be rejected"
            );
        }
    }

    #[test]
    fn revisions_are_deterministic_and_identity_is_separate() {
        let definition = compiled_workflow();
        let left = WorkflowRevision::new(
            WorkflowIdentity::new(
                "repo-1",
                WorkflowPath::parse("/.scope/runs/test.yml").unwrap(),
            )
            .unwrap(),
            definition.clone(),
        )
        .unwrap();
        let right = WorkflowRevision::new(
            WorkflowIdentity::new(
                "repo-2",
                WorkflowPath::parse("/.scope/runs/other.yml").unwrap(),
            )
            .unwrap(),
            definition,
        )
        .unwrap();

        assert_eq!(left.digest(), right.digest());
        assert_ne!(left.workflow(), right.workflow());
        assert_eq!(left.digest().len(), 64);
    }

    #[test]
    fn persisted_definitions_revalidate_invariants() {
        let definition = compiled_workflow();
        let json = serde_json::to_value(&definition).unwrap();
        assert_eq!(
            serde_json::from_value::<CompiledWorkflow>(json).unwrap(),
            definition
        );

        let mut invalid = serde_json::to_value(&definition).unwrap();
        invalid["timeout_seconds"] = serde_json::json!(0);
        assert!(serde_json::from_value::<CompiledWorkflow>(invalid).is_err());
    }

    #[test]
    fn compiled_workflow_enforces_behavior_invariants() {
        let duplicate_steps = CompiledWorkflow::new(
            "Test",
            WorkflowTriggers::new(true, false).unwrap(),
            RunnerSelector::named("linux-box").unwrap(),
            ContainerSpec::new("rust:1.90").unwrap(),
            60,
            vec![
                WorkflowStep::new("Test", "cargo test").unwrap(),
                WorkflowStep::new("Test", "cargo test --all").unwrap(),
            ],
        )
        .unwrap_err();
        assert!(matches!(
            duplicate_steps,
            WorkflowError::DuplicateStepName(name) if name == "Test"
        ));
        assert!(matches!(
            WorkflowTriggers::new(false, false),
            Err(WorkflowError::MissingTrigger)
        ));
        assert!(matches!(
            RunnerSelector::named("any"),
            Err(WorkflowError::InvalidRunnerName)
        ));
        assert!(matches!(
            CompiledWorkflow::new(
                "Test",
                WorkflowTriggers::new(true, false).unwrap(),
                RunnerSelector::Named("any".to_string()),
                ContainerSpec::new("rust:1.90").unwrap(),
                60,
                vec![WorkflowStep::new("Test", "cargo test").unwrap()],
            ),
            Err(WorkflowError::InvalidRunnerName)
        ));
    }
}
