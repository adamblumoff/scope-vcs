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
pub const MAX_WORKFLOW_ENVIRONMENT_VARIABLES: usize = 64;
pub const MAX_ENVIRONMENT_KEY_BYTES: usize = 128;
pub const MAX_ENVIRONMENT_VALUE_BYTES: usize = 64 * 1024;
pub const MAX_STEP_NAME_BYTES: usize = 100;
pub const MAX_STEP_COMMAND_BYTES: usize = 64 * 1024;
pub const MAX_WORKFLOW_TIMEOUT_SECONDS: u64 = 24 * 60 * 60;

const WORKFLOW_PATH_PREFIX: &str = "/.scope/runs/";
const WORKFLOW_DIGEST_VERSION: u8 = 4;

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
    #[error("workflow job cache path {0:?} overlaps another cache mount")]
    OverlappingCachePath(String),
    #[error(
        "workflow job cannot define more than {MAX_WORKFLOW_ENVIRONMENT_VARIABLES} environment variables"
    )]
    TooManyEnvironmentVariables,
    #[error(
        "workflow environment key must be a shell variable name between 1 and {MAX_ENVIRONMENT_KEY_BYTES} bytes"
    )]
    InvalidEnvironmentKey,
    #[error(
        "workflow environment value cannot exceed {MAX_ENVIRONMENT_VALUE_BYTES} bytes or contain a null byte"
    )]
    InvalidEnvironmentValue,
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
    environment: BTreeMap<String, String>,
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
            .find(|pair| pair[0].as_str() == pair[1].as_str())
            .map(|pair| pair[0].as_str().to_string())
        {
            return Err(WorkflowError::DuplicateCacheName(duplicate));
        }
        for (index, cache) in caches.iter().enumerate() {
            let path = std::path::Path::new(cache.mount_path());
            if caches[..index].iter().any(|existing| {
                let existing = std::path::Path::new(existing.mount_path());
                path.starts_with(existing) || existing.starts_with(path)
            }) {
                return Err(WorkflowError::OverlappingCachePath(
                    cache.mount_path().to_string(),
                ));
            }
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
            environment: BTreeMap::new(),
            steps,
        })
    }

    pub fn with_environment(
        mut self,
        environment: BTreeMap<String, String>,
    ) -> Result<Self, WorkflowError> {
        validate_environment(&environment)?;
        self.environment = environment;
        Ok(self)
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

    pub fn environment(&self) -> &BTreeMap<String, String> {
        &self.environment
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
    caches: Vec<PersistedWorkflowCache>,
    environment: BTreeMap<String, String>,
    steps: Vec<PersistedWorkflowStep>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PersistedWorkflowCache {
    name: String,
    path: String,
}

impl<'de> Deserialize<'de> for WorkflowJob {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let job = PersistedWorkflowJob::deserialize(deserializer)?;
        let id = WorkflowJobId::parse(job.id).map_err(D::Error::custom)?;
        let needs = job
            .needs
            .into_iter()
            .map(WorkflowJobId::parse)
            .collect::<Result<Vec<_>, _>>()
            .map_err(D::Error::custom)?;
        let runner = match job.runner {
            PersistedRunnerSelector::Any => RunnerSelector::Any,
            PersistedRunnerSelector::Named(name) => {
                RunnerSelector::named(name).map_err(D::Error::custom)?
            }
        };
        let container = ContainerSpec::new(job.container.image).map_err(D::Error::custom)?;
        let caches = job
            .caches
            .into_iter()
            .map(|cache| WorkflowCache::new(cache.name, cache.path))
            .collect::<Result<Vec<_>, _>>()
            .map_err(D::Error::custom)?;
        let steps = job
            .steps
            .into_iter()
            .map(|step| WorkflowStep::new(step.name, step.run))
            .collect::<Result<Vec<_>, _>>()
            .map_err(D::Error::custom)?;
        let environment = job.environment;
        WorkflowJob::new(
            id,
            needs,
            runner,
            container,
            job.timeout_seconds,
            caches,
            steps,
        )
        .and_then(|job| job.with_environment(environment))
        .map_err(D::Error::custom)
    }
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
                    .map(|cache| WorkflowCache::new(cache.name, cache.path))
                    .collect::<Result<Vec<_>, _>>()?;
                let steps = job
                    .steps
                    .into_iter()
                    .map(|step| WorkflowStep::new(step.name, step.run))
                    .collect::<Result<Vec<_>, _>>()?;
                let environment = job.environment;
                WorkflowJob::new(
                    id,
                    needs,
                    runner,
                    container,
                    job.timeout_seconds,
                    caches,
                    steps,
                )
                .and_then(|job| job.with_environment(environment))
            })
            .collect::<Result<Vec<_>, WorkflowError>>()
            .map_err(D::Error::custom)?;
        Self::new(persisted.name, triggers, jobs).map_err(D::Error::custom)
    }
}

fn validate_environment(environment: &BTreeMap<String, String>) -> Result<(), WorkflowError> {
    if environment.len() > MAX_WORKFLOW_ENVIRONMENT_VARIABLES {
        return Err(WorkflowError::TooManyEnvironmentVariables);
    }
    for (key, value) in environment {
        let mut bytes = key.bytes();
        if key.len() > MAX_ENVIRONMENT_KEY_BYTES
            || !bytes
                .next()
                .is_some_and(|byte| byte.is_ascii_alphabetic() || byte == b'_')
            || !bytes.all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
        {
            return Err(WorkflowError::InvalidEnvironmentKey);
        }
        if value.len() > MAX_ENVIRONMENT_VALUE_BYTES || value.as_bytes().contains(&0) {
            return Err(WorkflowError::InvalidEnvironmentValue);
        }
    }
    Ok(())
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

    /// Returns the job only when this workflow contains exactly one job.
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
mod tests;
