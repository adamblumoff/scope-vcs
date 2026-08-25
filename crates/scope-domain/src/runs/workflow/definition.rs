use super::{error::WorkflowError, validation::is_kebab_name};
use crate::runs::cache::definition::{CacheKeyInputs, WorkflowCache};
use serde::{Deserialize, Deserializer, Serialize, de::Error as _};
use std::collections::{BTreeMap, BTreeSet};

pub const MAX_WORKFLOW_NAME_BYTES: usize = 100;
pub const MAX_WORKFLOW_JOB_ID_BYTES: usize = 64;
pub const MAX_WORKFLOW_JOBS: usize = 64;
pub const MAX_CONTAINER_IMAGE_BYTES: usize = 512;
pub const MAX_WORKFLOW_STEPS: usize = 64;
pub const MAX_WORKFLOW_CACHES: usize = 16;
pub const MAX_WORKFLOW_ENVIRONMENT_VARIABLES: usize = 64;
pub const MAX_WORKFLOW_ENVIRONMENT_BYTES: usize = 64 * 1024;
pub const MAX_ENVIRONMENT_KEY_BYTES: usize = 128;
pub const MAX_ENVIRONMENT_VALUE_BYTES: usize = 64 * 1024;
pub const MAX_STEP_NAME_BYTES: usize = 100;
pub const MAX_STEP_COMMAND_BYTES: usize = 64 * 1024;
pub const MAX_WORKFLOW_TIMEOUT_SECONDS: u64 = 24 * 60 * 60;

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
pub struct ContainerSpec {
    image: String,
}

impl ContainerSpec {
    pub fn new(image: impl Into<String>) -> Result<Self, WorkflowError> {
        let image = image.into();
        let Some((repository, digest)) = image.rsplit_once("@sha256:") else {
            return Err(WorkflowError::InvalidContainerImage);
        };
        if repository.is_empty()
            || image.len() > MAX_CONTAINER_IMAGE_BYTES
            || repository.contains('@')
            || image.chars().any(char::is_whitespace)
            || digest.len() != 64
            || !digest.bytes().all(|byte| byte.is_ascii_hexdigit())
        {
            return Err(WorkflowError::InvalidContainerImage);
        }
        Ok(Self {
            image: format!("{repository}@sha256:{}", digest.to_ascii_lowercase()),
        })
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
        container: ContainerSpec,
        timeout_seconds: u64,
        mut caches: Vec<WorkflowCache>,
        environment: BTreeMap<String, String>,
        steps: Vec<WorkflowStep>,
    ) -> Result<Self, WorkflowError> {
        if timeout_seconds == 0 || timeout_seconds > MAX_WORKFLOW_TIMEOUT_SECONDS {
            return Err(WorkflowError::InvalidTimeout);
        }
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
        validate_environment(&environment)?;
        for input in caches.iter().flat_map(|cache| {
            cache
                .compatibility_inputs()
                .environment()
                .iter()
                .chain(cache.exact_inputs().environment())
        }) {
            if !environment.contains_key(input) {
                return Err(WorkflowError::UndeclaredCacheEnvironmentInput(
                    input.clone(),
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
            container,
            timeout_seconds,
            caches,
            environment,
            steps,
        })
    }

    pub fn id(&self) -> &WorkflowJobId {
        &self.id
    }

    pub fn needs(&self) -> &[WorkflowJobId] {
        &self.needs
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
#[serde(deny_unknown_fields)]
struct PersistedContainerSpec {
    image: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PersistedWorkflowJob {
    id: String,
    needs: Vec<String>,
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
    format: String,
    compatibility: PersistedCacheKeyInputs,
    exact: PersistedCacheKeyInputs,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PersistedCacheKeyInputs {
    files: Vec<String>,
    environment: Vec<String>,
    source: bool,
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
        let container = ContainerSpec::new(job.container.image).map_err(D::Error::custom)?;
        let caches = job
            .caches
            .into_iter()
            .map(|cache| {
                WorkflowCache::new(
                    cache.name,
                    cache.path,
                    cache.format,
                    CacheKeyInputs::new(
                        cache.compatibility.files,
                        cache.compatibility.environment,
                        cache.compatibility.source,
                    )?,
                    CacheKeyInputs::new(
                        cache.exact.files,
                        cache.exact.environment,
                        cache.exact.source,
                    )?,
                )
            })
            .collect::<Result<Vec<_>, _>>()
            .map_err(D::Error::custom)?;
        let steps = job
            .steps
            .into_iter()
            .map(|step| WorkflowStep::new(step.name, step.run))
            .collect::<Result<Vec<_>, _>>()
            .map_err(D::Error::custom)?;
        WorkflowJob::new(
            id,
            needs,
            container,
            job.timeout_seconds,
            caches,
            job.environment,
            steps,
        )
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
                let container = ContainerSpec::new(job.container.image)?;
                let caches = job
                    .caches
                    .into_iter()
                    .map(|cache| {
                        WorkflowCache::new(
                            cache.name,
                            cache.path,
                            cache.format,
                            CacheKeyInputs::new(
                                cache.compatibility.files,
                                cache.compatibility.environment,
                                cache.compatibility.source,
                            )?,
                            CacheKeyInputs::new(
                                cache.exact.files,
                                cache.exact.environment,
                                cache.exact.source,
                            )?,
                        )
                    })
                    .collect::<Result<Vec<_>, _>>()?;
                let steps = job
                    .steps
                    .into_iter()
                    .map(|step| WorkflowStep::new(step.name, step.run))
                    .collect::<Result<Vec<_>, _>>()?;
                WorkflowJob::new(
                    id,
                    needs,
                    container,
                    job.timeout_seconds,
                    caches,
                    job.environment,
                    steps,
                )
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
    let mut total_bytes = 0usize;
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
        total_bytes = total_bytes.saturating_add(key.len() + 1 + value.len());
    }
    if total_bytes > MAX_WORKFLOW_ENVIRONMENT_BYTES {
        return Err(WorkflowError::EnvironmentTooLarge);
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

#[cfg(test)]
mod tests {
    use super::*;

    const DIGEST: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";

    #[test]
    fn container_images_must_be_immutable() {
        assert!(ContainerSpec::new("rust:1.90").is_err());
        let image = ContainerSpec::new(format!("rust:1.90@sha256:{DIGEST}")).unwrap();
        assert_eq!(image.image(), format!("rust:1.90@sha256:{DIGEST}"));
    }

    #[test]
    fn cloud_workflow_preserves_jobs_dependencies_and_steps() {
        let build = WorkflowJob::new(
            WorkflowJobId::parse("build").unwrap(),
            vec![],
            ContainerSpec::new(format!("rust@sha256:{DIGEST}")).unwrap(),
            600,
            vec![],
            BTreeMap::new(),
            vec![WorkflowStep::new("Build", "cargo build").unwrap()],
        )
        .unwrap();
        let test = WorkflowJob::new(
            WorkflowJobId::parse("test").unwrap(),
            vec![WorkflowJobId::parse("build").unwrap()],
            ContainerSpec::new(format!("rust@sha256:{DIGEST}")).unwrap(),
            600,
            vec![],
            BTreeMap::new(),
            vec![WorkflowStep::new("Test", "cargo test").unwrap()],
        )
        .unwrap();
        let workflow = CompiledWorkflow::new(
            "Checks",
            WorkflowTriggers::new(true, true).unwrap(),
            vec![test, build],
        )
        .unwrap();
        assert_eq!(workflow.serial_jobs()[0].id().as_str(), "build");
        assert_eq!(workflow.serial_jobs()[1].id().as_str(), "test");
    }

    #[test]
    fn job_construction_rejects_undeclared_cache_environment_inputs() {
        let cache = WorkflowCache::new(
            "cargo",
            "/scope/cache/cargo",
            "v1",
            CacheKeyInputs::new(vec![], vec!["CARGO_INCREMENTAL".to_string()], false).unwrap(),
            CacheKeyInputs::new(vec![], vec![], false).unwrap(),
        )
        .unwrap();

        assert!(matches!(
            WorkflowJob::new(
                WorkflowJobId::parse("build").unwrap(),
                vec![],
                ContainerSpec::new(format!("rust@sha256:{DIGEST}")).unwrap(),
                600,
                vec![cache],
                BTreeMap::new(),
                vec![WorkflowStep::new("Build", "cargo build").unwrap()],
            ),
            Err(WorkflowError::UndeclaredCacheEnvironmentInput(name))
                if name == "CARGO_INCREMENTAL"
        ));
    }
}
