use scope_domain::runs::{
    cache::WorkflowCache,
    workflow::{
        CompiledWorkflow, ContainerSpec, RunnerSelector, WorkflowError, WorkflowIdentity,
        WorkflowJob, WorkflowJobId, WorkflowPath, WorkflowRevision, WorkflowStep, WorkflowTriggers,
    },
};
use serde::{Deserialize, Deserializer, de::MapAccess, de::Visitor};
use std::{
    collections::{BTreeMap, BTreeSet},
    fmt,
};
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
    let caches = raw
        .caches
        .into_iter()
        .map(|cache| WorkflowCache::new(cache.name, cache.path).map_err(WorkflowError::from))
        .collect::<Result<Vec<_>, _>>()?;
    let environment = raw.environment;
    let jobs = raw
        .jobs
        .0
        .into_iter()
        .map(|(id, job)| {
            let id = WorkflowJobId::parse(id)?;
            let needs = job
                .needs
                .into_iter()
                .map(WorkflowJobId::parse)
                .collect::<Result<Vec<_>, _>>()?;
            let job_runner = match job.runs_on {
                Some(name) if name == "any" => RunnerSelector::Any,
                Some(name) => RunnerSelector::named(name)?,
                None => runner.clone(),
            };
            let job_container = match job.container {
                Some(container) => ContainerSpec::new(container.image)?,
                None => container.clone(),
            };
            let job_timeout_seconds = match job.timeout {
                Some(timeout) => parse_timeout_seconds(&timeout)?,
                None => timeout_seconds,
            };
            let job_caches = match job.caches {
                Some(caches) => caches
                    .into_iter()
                    .map(|cache| {
                        WorkflowCache::new(cache.name, cache.path).map_err(WorkflowError::from)
                    })
                    .collect::<Result<Vec<_>, _>>()?,
                None => caches.clone(),
            };
            let mut job_environment = environment.clone();
            job_environment.extend(job.environment);
            let steps = job
                .steps
                .into_iter()
                .map(|step| WorkflowStep::new(step.name, step.run))
                .collect::<Result<Vec<_>, _>>()?;
            WorkflowJob::new(
                id,
                needs,
                job_runner,
                job_container,
                job_timeout_seconds,
                job_caches,
                steps,
            )
            .and_then(|job| job.with_environment(job_environment))
            .map_err(RunConfigError::from)
        })
        .collect::<Result<Vec<_>, _>>()?;
    let definition = CompiledWorkflow::new(raw.name, triggers, jobs)?;
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
    #[serde(default)]
    caches: Vec<RawCache>,
    #[serde(default, rename = "env")]
    environment: BTreeMap<String, String>,
    jobs: RawJobs,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawCache {
    name: String,
    path: String,
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
struct RawJob {
    #[serde(default)]
    needs: Vec<String>,
    #[serde(default, rename = "runs-on")]
    runs_on: Option<String>,
    #[serde(default)]
    container: Option<RawContainer>,
    #[serde(default)]
    timeout: Option<String>,
    #[serde(default)]
    caches: Option<Vec<RawCache>>,
    #[serde(default, rename = "env")]
    environment: BTreeMap<String, String>,
    steps: Vec<RawStep>,
}

#[derive(Debug)]
struct RawJobs(Vec<(String, RawJob)>);

impl<'de> Deserialize<'de> for RawJobs {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        struct RawJobsVisitor;

        impl<'de> Visitor<'de> for RawJobsVisitor {
            type Value = RawJobs;

            fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str("a mapping of workflow job IDs to job definitions")
            }

            fn visit_map<A>(self, mut map: A) -> Result<Self::Value, A::Error>
            where
                A: MapAccess<'de>,
            {
                let mut jobs = Vec::with_capacity(map.size_hint().unwrap_or(0));
                while let Some(entry) = map.next_entry()? {
                    jobs.push(entry);
                }
                Ok(RawJobs(jobs))
            }
        }

        deserializer.deserialize_map(RawJobsVisitor)
    }
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
caches:
  - name: cargo-target
    path: /workspace/target
  - name: cargo
    path: /scope/cache/cargo
env:
  RUSTUP_TOOLCHAIN: stable
jobs:
  checks:
    env:
      TEST_MODE: strict
    steps:
      - name: Format
        run: cargo fmt --check
      - name: Test
        run: cargo test --workspace
"#;

    #[test]
    fn parses_and_normalizes_jobs_workflow() {
        let parsed = parse_workflow("/.scope/runs/test.yml", WORKFLOW.as_bytes()).unwrap();
        let definition = parsed.definition();

        assert_eq!(parsed.path().name(), "test");
        assert_eq!(definition.name(), "Test");
        assert!(definition.triggers().manual());
        assert!(definition.triggers().push_main());
        let job = definition.only_job().unwrap();
        assert_eq!(job.id().as_str(), "checks");
        assert_eq!(job.timeout_seconds(), 20 * 60);
        assert_eq!(job.container().image(), "rust:1.90");
        assert_eq!(job.environment()["RUSTUP_TOOLCHAIN"], "stable");
        assert_eq!(job.environment()["TEST_MODE"], "strict");
        assert_eq!(
            job.caches()
                .iter()
                .map(WorkflowCache::as_str)
                .collect::<Vec<_>>(),
            ["cargo", "cargo-target"]
        );
        assert_eq!(job.steps()[1].run(), "cargo test --workspace");
    }

    #[test]
    fn equivalent_yaml_has_the_same_revision_digest() {
        let compact = r#"
name: Test
on: { push: true, manual: true }
runs-on: any
container: { image: "rust:1.90" }
timeout: 1200s
caches:
  - { name: cargo, path: /scope/cache/cargo }
  - { name: cargo-target, path: /workspace/target }
env: { TEST_MODE: strict, RUSTUP_TOOLCHAIN: stable }
jobs:
  checks:
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
    fn cache_yaml_order_does_not_change_the_revision_digest() {
        let reversed =
            WORKFLOW.replace(
                "  - name: cargo-target\n    path: /workspace/target\n  - name: cargo\n    path: /scope/cache/cargo",
                "  - name: cargo\n    path: /scope/cache/cargo\n  - name: cargo-target\n    path: /workspace/target",
            );
        let first = parse_workflow("/.scope/runs/test.yml", WORKFLOW.as_bytes())
            .unwrap()
            .into_revision("repo-1")
            .unwrap();
        let second = parse_workflow("/.scope/runs/test.yml", reversed.as_bytes())
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
    fn scope_workflows_use_the_current_contract() {
        for (path, bytes) in [
            (
                "/.scope/runs/checks.yml",
                include_bytes!("../../../.scope/runs/checks.yml").as_slice(),
            ),
            (
                "/.scope/runs/v5-canary-cold-write.yml",
                include_bytes!("../../../.scope/runs/v5-canary-cold-write.yml").as_slice(),
            ),
            (
                "/.scope/runs/v5-canary-warm-read.yml",
                include_bytes!("../../../.scope/runs/v5-canary-warm-read.yml").as_slice(),
            ),
            (
                "/.scope/runs/v5-canary-evict.yml",
                include_bytes!("../../../.scope/runs/v5-canary-evict.yml").as_slice(),
            ),
        ] {
            parse_workflow(path, bytes).unwrap_or_else(|error| {
                panic!("{path} must follow the current workflow contract: {error}")
            });
        }
    }

    #[test]
    fn checked_in_cutover_workflows_are_canonical_canaries() {
        use scope_domain::runs::cutover::{
            RunnerProtocolCanaryPhase, validate_runner_protocol_canary_workflow,
        };

        for (phase, path, bytes) in [
            (
                RunnerProtocolCanaryPhase::ColdWrite,
                "/.scope/runs/v5-canary-cold-write.yml",
                include_bytes!("../../../.scope/runs/v5-canary-cold-write.yml").as_slice(),
            ),
            (
                RunnerProtocolCanaryPhase::WarmRead,
                "/.scope/runs/v5-canary-warm-read.yml",
                include_bytes!("../../../.scope/runs/v5-canary-warm-read.yml").as_slice(),
            ),
            (
                RunnerProtocolCanaryPhase::Evict,
                "/.scope/runs/v5-canary-evict.yml",
                include_bytes!("../../../.scope/runs/v5-canary-evict.yml").as_slice(),
            ),
        ] {
            let parsed = parse_workflow(path, bytes).unwrap();
            validate_runner_protocol_canary_workflow(parsed.definition(), phase)
                .unwrap_or_else(|error| panic!("{path} must be a canonical canary: {error}"));
        }
    }

    #[test]
    fn caches_default_to_empty_and_reject_invalid_or_ambiguous_mounts() {
        let missing = WORKFLOW.replace(
            "caches:\n  - name: cargo-target\n    path: /workspace/target\n  - name: cargo\n    path: /scope/cache/cargo\n",
            "",
        );
        assert!(
            parse_workflow("/.scope/runs/test.yml", missing.as_bytes())
                .unwrap()
                .definition()
                .only_job()
                .unwrap()
                .caches()
                .is_empty()
        );

        let invalid = WORKFLOW.replace("name: cargo-target", "name: Cargo_Target");
        assert!(matches!(
            parse_workflow("/.scope/runs/test.yml", invalid.as_bytes()),
            Err(RunConfigError::InvalidWorkflow(
                WorkflowError::InvalidCache(_)
            ))
        ));

        let duplicate = WORKFLOW.replace("name: cargo\n", "name: cargo-target\n");
        assert!(matches!(
            parse_workflow("/.scope/runs/test.yml", duplicate.as_bytes()),
            Err(RunConfigError::InvalidWorkflow(
                WorkflowError::DuplicateCacheName(name)
            )) if name == "cargo-target"
        ));

        let old_list = WORKFLOW.replace(
            "  - name: cargo-target\n    path: /workspace/target\n  - name: cargo\n    path: /scope/cache/cargo",
            "  - cargo-target\n  - cargo",
        );
        assert!(matches!(
            parse_workflow("/.scope/runs/test.yml", old_list.as_bytes()),
            Err(RunConfigError::InvalidYaml(_))
        ));

        let overlapping = WORKFLOW.replace(
            "path: /scope/cache/cargo",
            "path: /workspace/target/registry",
        );
        assert!(matches!(
            parse_workflow("/.scope/runs/test.yml", overlapping.as_bytes()),
            Err(RunConfigError::InvalidWorkflow(
                WorkflowError::OverlappingCachePath(_)
            ))
        ));
    }

    #[test]
    fn environment_is_strict_and_job_values_override_workflow_values() {
        let parsed = parse_workflow("/.scope/runs/test.yml", WORKFLOW.as_bytes()).unwrap();
        let environment = parsed.definition().only_job().unwrap().environment();
        assert_eq!(environment["RUSTUP_TOOLCHAIN"], "stable");
        assert_eq!(environment["TEST_MODE"], "strict");

        for invalid in ["1INVALID", "WITH-DASH"] {
            let workflow = WORKFLOW.replace("RUSTUP_TOOLCHAIN:", &format!("{invalid}:"));
            assert!(matches!(
                parse_workflow("/.scope/runs/test.yml", workflow.as_bytes()),
                Err(RunConfigError::InvalidWorkflow(
                    WorkflowError::InvalidEnvironmentKey
                ))
            ));
        }
    }

    #[test]
    fn jobs_inherit_and_can_override_workflow_runtime_defaults() {
        let workflow = r#"
name: Graph
on: { manual: true }
runs-on: remote-linux
container: { image: rust:1.90 }
timeout: 20m
caches: [{ name: cargo, path: /scope/cache/cargo }]
env: { SHARED: workflow, WORKFLOW_ONLY: yes }
jobs:
  backend:
    env: { SHARED: backend }
    steps:
      - { name: Backend, run: cargo test }
  web:
    needs: [backend]
    runs-on: browser-runner
    container: { image: node:24 }
    timeout: 5m
    caches: []
    steps:
      - { name: Web, run: pnpm test }
"#;
        let parsed = parse_workflow("/.scope/runs/graph.yml", workflow.as_bytes()).unwrap();
        let definition = parsed.definition();
        let backend = definition
            .job(&WorkflowJobId::parse("backend").unwrap())
            .unwrap();
        assert_eq!(
            backend.runner(),
            &RunnerSelector::named("remote-linux").unwrap()
        );
        assert_eq!(backend.container().image(), "rust:1.90");
        assert_eq!(backend.timeout_seconds(), 20 * 60);
        assert_eq!(backend.caches()[0].as_str(), "cargo");
        assert_eq!(backend.environment()["SHARED"], "backend");
        assert_eq!(backend.environment()["WORKFLOW_ONLY"], "yes");

        let web = definition
            .job(&WorkflowJobId::parse("web").unwrap())
            .unwrap();
        assert_eq!(web.needs()[0].as_str(), "backend");
        assert_eq!(
            web.runner(),
            &RunnerSelector::named("browser-runner").unwrap()
        );
        assert_eq!(web.container().image(), "node:24");
        assert_eq!(web.timeout_seconds(), 5 * 60);
        assert!(web.caches().is_empty());
        assert_eq!(web.environment()["SHARED"], "workflow");
    }

    #[test]
    fn rejects_invalid_graphs_and_the_removed_flat_step_schema() {
        let missing =
            WORKFLOW.replace("jobs:\n  checks:", "jobs:\n  checks:\n    needs: [missing]");
        assert!(matches!(
            parse_workflow("/.scope/runs/test.yml", missing.as_bytes()),
            Err(RunConfigError::InvalidWorkflow(
                WorkflowError::MissingDependency { .. }
            ))
        ));

        let flat = WORKFLOW.replace("jobs:\n  checks:\n    ", "");
        assert!(matches!(
            parse_workflow("/.scope/runs/test.yml", flat.as_bytes()),
            Err(RunConfigError::InvalidYaml(_))
        ));

        let duplicate_job = format!(
            "{WORKFLOW}  checks:\n    steps:\n      - {{ name: Duplicate, run: 'true' }}\n"
        );
        assert!(matches!(
            parse_workflow("/.scope/runs/test.yml", duplicate_job.as_bytes()),
            Err(RunConfigError::InvalidWorkflow(
                WorkflowError::DuplicateJobId(id)
            )) if id == "checks"
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
