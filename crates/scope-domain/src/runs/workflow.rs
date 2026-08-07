use super::{
    cache::{CacheError, WorkflowCache},
    runner::RunnerName,
};
use serde::{Deserialize, Deserializer, Serialize, de::Error as _};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use thiserror::Error;

pub const MAX_WORKFLOW_NAME_BYTES: usize = 100;
pub const MAX_WORKFLOW_PATH_NAME_BYTES: usize = 64;
pub const MAX_WORKFLOW_JOB_ID_BYTES: usize = 64;
pub const MAX_WORKFLOW_JOBS: usize = 64;
pub const MAX_CONTAINER_IMAGE_BYTES: usize = 512;
pub const MAX_WORKFLOW_STEPS: usize = 64;
pub const MAX_WORKFLOW_CACHES: usize = 16;
pub const MAX_STEP_NAME_BYTES: usize = 100;
pub const MAX_STEP_COMMAND_BYTES: usize = 64 * 1024;
pub const MAX_WORKFLOW_TIMEOUT_SECONDS: u64 = 24 * 60 * 60;

const WORKFLOW_PATH_PREFIX: &str = "/.scope/runs/";
const WORKFLOW_DIGEST_VERSION: u8 = 3;

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
    #[error("workflow must contain at least one job")]
    MissingJobs,
    #[error("workflow cannot contain more than {MAX_WORKFLOW_JOBS} jobs")]
    TooManyJobs,
    #[error(
        "workflow job id must be a lowercase kebab name between 1 and {MAX_WORKFLOW_JOB_ID_BYTES} bytes"
    )]
    InvalidJobId,
    #[error("workflow job id {0:?} is duplicated")]
    DuplicateJobId(String),
    #[error("workflow job {job:?} depends on itself")]
    SelfDependency { job: String },
    #[error("workflow job {job:?} names dependency {dependency:?} more than once")]
    DuplicateDependency { job: String, dependency: String },
    #[error("workflow job {job:?} depends on missing job {dependency:?}")]
    MissingDependency { job: String, dependency: String },
    #[error("workflow job dependencies contain a cycle")]
    DependencyCycle,
    #[error("workflow job must contain at least one step")]
    MissingSteps,
    #[error("workflow job cannot contain more than {MAX_WORKFLOW_STEPS} steps")]
    TooManySteps,
    #[error("workflow job cannot contain more than {MAX_WORKFLOW_CACHES} caches")]
    TooManyCaches,
    #[error("workflow job cache name {0:?} is duplicated")]
    DuplicateCacheName(String),
    #[error("workflow step name must contain between 1 and {MAX_STEP_NAME_BYTES} bytes")]
    InvalidStepName,
    #[error("workflow step name {0:?} is duplicated")]
    DuplicateStepName(String),
    #[error("workflow step command must contain between 1 and {MAX_STEP_COMMAND_BYTES} bytes")]
    InvalidStepCommand,
    #[error("workflow revision digest could not be serialized: {0}")]
    Digest(serde_json::Error),
    #[error(transparent)]
    InvalidCache(#[from] CacheError),
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
        if !is_kebab_name(stem, MAX_WORKFLOW_PATH_NAME_BYTES) {
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

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(transparent)]
pub struct WorkflowJobId(String);

impl WorkflowJobId {
    pub fn parse(id: impl Into<String>) -> Result<Self, WorkflowError> {
        let id = id.into();
        if !is_kebab_name(&id, MAX_WORKFLOW_JOB_ID_BYTES) {
            return Err(WorkflowError::InvalidJobId);
        }
        Ok(Self(id))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct WorkflowJob {
    id: WorkflowJobId,
    needs: Vec<WorkflowJobId>,
    runner: RunnerSelector,
    container: ContainerSpec,
    timeout_seconds: u64,
    caches: Vec<WorkflowCache>,
    steps: Vec<WorkflowStep>,
}

impl WorkflowJob {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        id: WorkflowJobId,
        mut needs: Vec<WorkflowJobId>,
        runner: RunnerSelector,
        container: ContainerSpec,
        timeout_seconds: u64,
        mut caches: Vec<WorkflowCache>,
        steps: Vec<WorkflowStep>,
    ) -> Result<Self, WorkflowError> {
        if timeout_seconds == 0 || timeout_seconds > MAX_WORKFLOW_TIMEOUT_SECONDS {
            return Err(WorkflowError::InvalidTimeout);
        }
        runner.validate()?;
        needs.sort();
        if needs.binary_search(&id).is_ok() {
            return Err(WorkflowError::SelfDependency {
                job: id.as_str().to_string(),
            });
        }
        if let Some(duplicate) = needs
            .windows(2)
            .find(|pair| pair[0] == pair[1])
            .map(|pair| pair[0].as_str().to_string())
        {
            return Err(WorkflowError::DuplicateDependency {
                job: id.as_str().to_string(),
                dependency: duplicate,
            });
        }
        if caches.len() > MAX_WORKFLOW_CACHES {
            return Err(WorkflowError::TooManyCaches);
        }
        caches.sort();
        if let Some(duplicate) = caches
            .windows(2)
            .find(|pair| pair[0] == pair[1])
            .map(|pair| pair[0].as_str().to_string())
        {
            return Err(WorkflowError::DuplicateCacheName(duplicate));
        }
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
            id,
            needs,
            runner,
            container,
            timeout_seconds,
            caches,
            steps,
        })
    }

    pub fn id(&self) -> &WorkflowJobId {
        &self.id
    }

    pub fn needs(&self) -> &[WorkflowJobId] {
        &self.needs
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

    pub fn caches(&self) -> &[WorkflowCache] {
        &self.caches
    }

    pub fn steps(&self) -> &[WorkflowStep] {
        &self.steps
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct CompiledWorkflow {
    name: String,
    triggers: WorkflowTriggers,
    jobs: Vec<WorkflowJob>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PersistedCompiledWorkflow {
    name: String,
    triggers: PersistedWorkflowTriggers,
    jobs: Vec<PersistedWorkflowJob>,
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
struct PersistedWorkflowJob {
    id: String,
    needs: Vec<String>,
    runner: PersistedRunnerSelector,
    container: PersistedContainerSpec,
    timeout_seconds: u64,
    caches: Vec<String>,
    steps: Vec<PersistedWorkflowStep>,
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
        let jobs = persisted
            .jobs
            .into_iter()
            .map(|job| {
                let id = WorkflowJobId::parse(job.id)?;
                let needs = job
                    .needs
                    .into_iter()
                    .map(WorkflowJobId::parse)
                    .collect::<Result<Vec<_>, _>>()?;
                let runner = match job.runner {
                    PersistedRunnerSelector::Any => RunnerSelector::Any,
                    PersistedRunnerSelector::Named(name) => RunnerSelector::named(name)?,
                };
                let container = ContainerSpec::new(job.container.image)?;
                let caches = job
                    .caches
                    .into_iter()
                    .map(WorkflowCache::parse)
                    .collect::<Result<Vec<_>, _>>()?;
                let steps = job
                    .steps
                    .into_iter()
                    .map(|step| WorkflowStep::new(step.name, step.run))
                    .collect::<Result<Vec<_>, _>>()?;
                WorkflowJob::new(
                    id,
                    needs,
                    runner,
                    container,
                    job.timeout_seconds,
                    caches,
                    steps,
                )
            })
            .collect::<Result<Vec<_>, WorkflowError>>()
            .map_err(D::Error::custom)?;
        Self::new(persisted.name, triggers, jobs).map_err(D::Error::custom)
    }
}

impl CompiledWorkflow {
    pub fn new(
        name: impl Into<String>,
        triggers: WorkflowTriggers,
        mut jobs: Vec<WorkflowJob>,
    ) -> Result<Self, WorkflowError> {
        let name = name.into();
        let name = name.trim();
        if name.is_empty() || name.len() > MAX_WORKFLOW_NAME_BYTES {
            return Err(WorkflowError::InvalidName);
        }
        if jobs.is_empty() {
            return Err(WorkflowError::MissingJobs);
        }
        if jobs.len() > MAX_WORKFLOW_JOBS {
            return Err(WorkflowError::TooManyJobs);
        }
        jobs.sort_by(|left, right| left.id.cmp(&right.id));
        if let Some(duplicate) = jobs
            .windows(2)
            .find(|pair| pair[0].id == pair[1].id)
            .map(|pair| pair[0].id.as_str().to_string())
        {
            return Err(WorkflowError::DuplicateJobId(duplicate));
        }
        validate_job_graph(&jobs)?;
        Ok(Self {
            name: name.to_string(),
            triggers,
            jobs,
        })
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn triggers(&self) -> &WorkflowTriggers {
        &self.triggers
    }

    pub fn jobs(&self) -> &[WorkflowJob] {
        &self.jobs
    }

    pub fn job(&self, id: &WorkflowJobId) -> Option<&WorkflowJob> {
        self.jobs
            .binary_search_by(|job| job.id.cmp(id))
            .ok()
            .map(|index| &self.jobs[index])
    }

    /// Temporary bridge for the run-level dispatch protocol. Multi-job workflows
    /// become dispatchable when attempts move beneath jobs.
    pub fn only_job(&self) -> Option<&WorkflowJob> {
        if self.jobs.len() == 1 {
            self.jobs.first()
        } else {
            None
        }
    }

    pub fn serial_jobs(&self) -> Vec<&WorkflowJob> {
        topological_job_indices(&self.jobs)
            .into_iter()
            .map(|index| &self.jobs[index])
            .collect()
    }
}

fn validate_job_graph(jobs: &[WorkflowJob]) -> Result<(), WorkflowError> {
    let ids = jobs
        .iter()
        .map(|job| job.id.clone())
        .collect::<BTreeSet<_>>();
    for job in jobs {
        if let Some(missing) = job
            .needs
            .iter()
            .find(|dependency| !ids.contains(*dependency))
        {
            return Err(WorkflowError::MissingDependency {
                job: job.id.as_str().to_string(),
                dependency: missing.as_str().to_string(),
            });
        }
    }
    if topological_job_indices(jobs).len() != jobs.len() {
        return Err(WorkflowError::DependencyCycle);
    }
    Ok(())
}

fn topological_job_indices(jobs: &[WorkflowJob]) -> Vec<usize> {
    let indices = jobs
        .iter()
        .enumerate()
        .map(|(index, job)| (job.id.clone(), index))
        .collect::<BTreeMap<_, _>>();
    let mut remaining_needs = jobs.iter().map(|job| job.needs.len()).collect::<Vec<_>>();
    let mut dependents = vec![Vec::new(); jobs.len()];
    for (job_index, job) in jobs.iter().enumerate() {
        for dependency in &job.needs {
            if let Some(dependency_index) = indices.get(dependency) {
                dependents[*dependency_index].push(job_index);
            }
        }
    }
    let mut ready = remaining_needs
        .iter()
        .enumerate()
        .filter_map(|(index, count)| (*count == 0).then_some(jobs[index].id.clone()))
        .collect::<BTreeSet<_>>();
    let mut order = Vec::with_capacity(jobs.len());
    while let Some(id) = ready.pop_first() {
        let index = indices[&id];
        order.push(index);
        for dependent in &dependents[index] {
            remaining_needs[*dependent] -= 1;
            if remaining_needs[*dependent] == 0 {
                ready.insert(jobs[*dependent].id.clone());
            }
        }
    }
    order
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

fn is_kebab_name(name: &str, max_bytes: usize) -> bool {
    !name.is_empty()
        && name.len() <= max_bytes
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

    fn job(
        id: &str,
        needs: &[&str],
        caches: Vec<WorkflowCache>,
        steps: Vec<WorkflowStep>,
    ) -> WorkflowJob {
        WorkflowJob::new(
            WorkflowJobId::parse(id).unwrap(),
            needs
                .iter()
                .map(|need| WorkflowJobId::parse(*need).unwrap())
                .collect(),
            RunnerSelector::Any,
            ContainerSpec::new("rust:1.90").unwrap(),
            20 * 60,
            caches,
            steps,
        )
        .unwrap()
    }

    fn compiled_workflow() -> CompiledWorkflow {
        CompiledWorkflow::new(
            "Test",
            WorkflowTriggers::new(true, true).unwrap(),
            vec![job(
                "checks",
                &[],
                vec![
                    WorkflowCache::parse("cargo-target").unwrap(),
                    WorkflowCache::parse("cargo").unwrap(),
                ],
                vec![
                    WorkflowStep::new("Format", "cargo fmt --check").unwrap(),
                    WorkflowStep::new("Test", "cargo test --workspace").unwrap(),
                ],
            )],
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
        assert_eq!(
            left.definition()
                .only_job()
                .unwrap()
                .caches()
                .iter()
                .map(WorkflowCache::as_str)
                .collect::<Vec<_>>(),
            ["cargo", "cargo-target"]
        );
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
        invalid["jobs"][0]["timeout_seconds"] = serde_json::json!(0);
        assert!(serde_json::from_value::<CompiledWorkflow>(invalid).is_err());
    }

    #[test]
    fn compiled_workflow_enforces_behavior_invariants() {
        let duplicate_steps = WorkflowJob::new(
            WorkflowJobId::parse("checks").unwrap(),
            vec![],
            RunnerSelector::named("linux-box").unwrap(),
            ContainerSpec::new("rust:1.90").unwrap(),
            60,
            vec![],
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
            WorkflowJob::new(
                WorkflowJobId::parse("checks").unwrap(),
                vec![],
                RunnerSelector::Named("any".to_string()),
                ContainerSpec::new("rust:1.90").unwrap(),
                60,
                vec![],
                vec![WorkflowStep::new("Test", "cargo test").unwrap()],
            ),
            Err(WorkflowError::InvalidRunnerName)
        ));

        let dependency = WorkflowJobId::parse("checks").unwrap();
        assert!(matches!(
            WorkflowJob::new(
                dependency.clone(),
                vec![dependency.clone()],
                RunnerSelector::Any,
                ContainerSpec::new("rust:1.90").unwrap(),
                60,
                vec![],
                vec![WorkflowStep::new("Test", "cargo test").unwrap()],
            ),
            Err(WorkflowError::SelfDependency { job }) if job == "checks"
        ));
        assert!(matches!(
            WorkflowJob::new(
                WorkflowJobId::parse("web").unwrap(),
                vec![dependency.clone(), dependency],
                RunnerSelector::Any,
                ContainerSpec::new("rust:1.90").unwrap(),
                60,
                vec![],
                vec![WorkflowStep::new("Test", "cargo test").unwrap()],
            ),
            Err(WorkflowError::DuplicateDependency { job, dependency })
                if job == "web" && dependency == "checks"
        ));

        let duplicate_job = job(
            "checks",
            &[],
            vec![],
            vec![WorkflowStep::new("Test", "cargo test").unwrap()],
        );
        assert!(matches!(
            CompiledWorkflow::new(
                "Test",
                WorkflowTriggers::new(true, false).unwrap(),
                vec![duplicate_job.clone(), duplicate_job],
            ),
            Err(WorkflowError::DuplicateJobId(id)) if id == "checks"
        ));
    }

    #[test]
    fn cache_and_job_order_are_canonical_in_the_v3_digest() {
        let identity = || {
            WorkflowIdentity::new(
                "repo-1",
                WorkflowPath::parse("/.scope/runs/test.yml").unwrap(),
            )
            .unwrap()
        };
        let definition = |caches| {
            CompiledWorkflow::new(
                "Test",
                WorkflowTriggers::new(true, false).unwrap(),
                vec![job(
                    "checks",
                    &[],
                    caches,
                    vec![WorkflowStep::new("Test", "cargo test").unwrap()],
                )],
            )
            .unwrap()
        };
        let cargo = WorkflowCache::parse("cargo").unwrap();
        let target = WorkflowCache::parse("cargo-target").unwrap();
        let left =
            WorkflowRevision::new(identity(), definition(vec![target.clone(), cargo.clone()]))
                .unwrap();
        let right =
            WorkflowRevision::new(identity(), definition(vec![cargo.clone(), target])).unwrap();
        let without_cache = WorkflowRevision::new(identity(), definition(vec![])).unwrap();

        assert_eq!(left.digest(), right.digest());
        assert_ne!(left.digest(), without_cache.digest());
        assert!(matches!(
            WorkflowJob::new(
                WorkflowJobId::parse("checks").unwrap(),
                vec![],
                RunnerSelector::Any,
                ContainerSpec::new("rust:1.90").unwrap(),
                60,
                vec![cargo.clone(), cargo],
                vec![WorkflowStep::new("Test", "cargo test").unwrap()],
            ),
            Err(WorkflowError::DuplicateCacheName(name)) if name == "cargo"
        ));
        let excessive = (0..=MAX_WORKFLOW_CACHES)
            .map(|index| WorkflowCache::parse(format!("cache-{index}")))
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
        assert!(matches!(
            WorkflowJob::new(
                WorkflowJobId::parse("checks").unwrap(),
                vec![],
                RunnerSelector::Any,
                ContainerSpec::new("rust:1.90").unwrap(),
                60,
                excessive,
                vec![WorkflowStep::new("Test", "cargo test").unwrap()],
            ),
            Err(WorkflowError::TooManyCaches)
        ));

        let first = CompiledWorkflow::new(
            "Graph",
            WorkflowTriggers::new(true, false).unwrap(),
            vec![
                job(
                    "web",
                    &[],
                    vec![],
                    vec![WorkflowStep::new("Web", "true").unwrap()],
                ),
                job(
                    "backend",
                    &[],
                    vec![],
                    vec![WorkflowStep::new("Backend", "true").unwrap()],
                ),
            ],
        )
        .unwrap();
        let second = CompiledWorkflow::new(
            "Graph",
            WorkflowTriggers::new(true, false).unwrap(),
            first.jobs().iter().cloned().rev().collect(),
        )
        .unwrap();
        assert_eq!(
            WorkflowRevision::new(identity(), first).unwrap().digest(),
            WorkflowRevision::new(identity(), second).unwrap().digest()
        );
    }

    #[test]
    fn job_graph_rejects_missing_dependencies_and_cycles() {
        let missing = CompiledWorkflow::new(
            "Graph",
            WorkflowTriggers::new(true, false).unwrap(),
            vec![job(
                "web",
                &["backend"],
                vec![],
                vec![WorkflowStep::new("Web", "true").unwrap()],
            )],
        )
        .unwrap_err();
        assert!(matches!(
            missing,
            WorkflowError::MissingDependency { job, dependency }
                if job == "web" && dependency == "backend"
        ));

        let cycle = CompiledWorkflow::new(
            "Graph",
            WorkflowTriggers::new(true, false).unwrap(),
            vec![
                job(
                    "backend",
                    &["web"],
                    vec![],
                    vec![WorkflowStep::new("Backend", "true").unwrap()],
                ),
                job(
                    "web",
                    &["backend"],
                    vec![],
                    vec![WorkflowStep::new("Web", "true").unwrap()],
                ),
            ],
        )
        .unwrap_err();
        assert!(matches!(cycle, WorkflowError::DependencyCycle));
    }

    #[test]
    fn serial_jobs_use_a_deterministic_topological_order() {
        let workflow = CompiledWorkflow::new(
            "Graph",
            WorkflowTriggers::new(true, false).unwrap(),
            vec![
                job(
                    "integration",
                    &["web", "backend"],
                    vec![],
                    vec![WorkflowStep::new("Integration", "true").unwrap()],
                ),
                job(
                    "web",
                    &[],
                    vec![],
                    vec![WorkflowStep::new("Web", "true").unwrap()],
                ),
                job(
                    "backend",
                    &[],
                    vec![],
                    vec![WorkflowStep::new("Backend", "true").unwrap()],
                ),
            ],
        )
        .unwrap();

        assert_eq!(
            workflow
                .serial_jobs()
                .into_iter()
                .map(|job| job.id().as_str())
                .collect::<Vec<_>>(),
            ["backend", "web", "integration"]
        );
        assert!(workflow.only_job().is_none());
    }

    #[test]
    fn persisted_flat_definitions_are_not_accepted() {
        let mut json = serde_json::to_value(compiled_workflow()).unwrap();
        let jobs = json.as_object_mut().unwrap().remove("jobs").unwrap();
        let job = &jobs[0];
        for field in ["runner", "container", "timeout_seconds", "caches", "steps"] {
            json[field] = job[field].clone();
        }
        assert!(serde_json::from_value::<CompiledWorkflow>(json).is_err());
    }
}
