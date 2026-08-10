use crate::error::DomainError;
use serde::{Deserialize, Deserializer, Serialize, de::Error as _};

use super::{
    run::PinnedContainerImage,
    runner::RUNNER_PROTOCOL_VERSION,
    workflow::{CompiledWorkflow, RunnerSelector},
};

pub const RUNNER_PROTOCOL_CANARY_CACHE_NAME: &str = "runner-protocol";
pub const RUNNER_PROTOCOL_CANARY_CACHE_PATH: &str = "/scope/cache/runner-protocol";
pub const RUNNER_PROTOCOL_CANARY_SENTINEL_PATH: &str =
    "/scope/cache/runner-protocol/v7-canary-sentinel";
pub const RUNNER_PROTOCOL_CANARY_SENTINEL_VALUE: &str = "scope-runner-protocol-v7";
pub const RUNNER_PROTOCOL_CANARY_TIMEOUT_SECONDS: u64 = 5 * 60;
pub const RUNNER_PROTOCOL_CANARY_COLD_WRITE_COMMAND: &str =
    "printf '%s' 'scope-runner-protocol-v7' > /scope/cache/runner-protocol/v7-canary-sentinel";
pub const RUNNER_PROTOCOL_CANARY_READ_COMMAND: &str =
    "test \"$(cat /scope/cache/runner-protocol/v7-canary-sentinel)\" = 'scope-runner-protocol-v7'";

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum RunnerProtocolCutoverState {
    V7Fenced,
    V7Open,
}

impl RunnerProtocolCutoverState {
    pub fn can_transition_to(self, next: Self) -> bool {
        matches!((self, next), (Self::V7Fenced, Self::V7Open))
    }

    pub fn allows_runner_registration(self, protocol_version: u32) -> bool {
        self.allows_runner_authentication(protocol_version)
    }

    pub fn allows_runner_authentication(self, protocol_version: u32) -> bool {
        protocol_version == RUNNER_PROTOCOL_VERSION
    }

    pub fn allows_workflow_writes(self) -> bool {
        self == Self::V7Open
    }

    pub fn allows_enqueue(self) -> bool {
        self.allows_workflow_writes()
    }

    pub fn allows_claim(self, protocol_version: u32) -> bool {
        self == Self::V7Open && protocol_version == RUNNER_PROTOCOL_VERSION
    }

    pub fn allows_attempt_operation(self, protocol_version: u32) -> bool {
        let _ = self;
        protocol_version == RUNNER_PROTOCOL_VERSION
    }

    pub fn allows_canary(self) -> bool {
        self == Self::V7Fenced
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RunnerProtocolCutover {
    state: RunnerProtocolCutoverState,
}

impl RunnerProtocolCutover {
    pub fn new() -> Self {
        Self {
            state: RunnerProtocolCutoverState::V7Fenced,
        }
    }

    pub fn restore(state: RunnerProtocolCutoverState) -> Self {
        Self { state }
    }

    pub fn state(&self) -> RunnerProtocolCutoverState {
        self.state
    }

    pub fn transition(&mut self, next: RunnerProtocolCutoverState) -> Result<bool, DomainError> {
        if next == self.state {
            return Ok(false);
        }
        if !self.state.can_transition_to(next) {
            return Err(DomainError::conflict(format!(
                "runner protocol cutover cannot transition from {:?} to {next:?}",
                self.state
            )));
        }
        self.state = next;
        Ok(true)
    }
}

impl Default for RunnerProtocolCutover {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(transparent)]
pub struct CanaryGeneration(u64);

impl CanaryGeneration {
    pub fn new(value: u64) -> Result<Self, DomainError> {
        if value == 0 {
            return Err(DomainError::invalid_input(
                "runner protocol canary generation must be greater than zero",
            ));
        }
        Ok(Self(value))
    }

    pub fn get(self) -> u64 {
        self.0
    }

    pub fn next(self) -> Result<Self, DomainError> {
        self.0
            .checked_add(1)
            .map(Self)
            .ok_or_else(|| DomainError::conflict("runner protocol canary generation overflow"))
    }
}

impl<'de> Deserialize<'de> for CanaryGeneration {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        Self::new(u64::deserialize(deserializer)?).map_err(D::Error::custom)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum RunnerProtocolCanaryPhase {
    ColdWrite,
    WarmRead,
    Evict,
}

impl RunnerProtocolCanaryPhase {
    pub fn workflow_name(self) -> &'static str {
        match self {
            Self::ColdWrite => "Runner protocol V7 cold-write canary",
            Self::WarmRead => "Runner protocol V7 warm-read canary",
            Self::Evict => "Runner protocol V7 eviction canary",
        }
    }

    pub fn step_name(self) -> &'static str {
        match self {
            Self::ColdWrite => "Write cache sentinel",
            Self::WarmRead => "Assert warm cache sentinel",
            Self::Evict => "Assert cache sentinel before eviction",
        }
    }

    pub fn step_command(self) -> &'static str {
        match self {
            Self::ColdWrite => RUNNER_PROTOCOL_CANARY_COLD_WRITE_COMMAND,
            Self::WarmRead | Self::Evict => RUNNER_PROTOCOL_CANARY_READ_COMMAND,
        }
    }

    pub fn evicts_cache_after_success(self) -> bool {
        self == Self::Evict
    }
}

pub fn validate_runner_protocol_canary_workflow(
    workflow: &CompiledWorkflow,
    phase: RunnerProtocolCanaryPhase,
) -> Result<(), DomainError> {
    let invalid = |detail: &str| {
        DomainError::invalid_input(format!(
            "runner protocol {} canary workflow is not canonical: {detail}",
            phase.workflow_name()
        ))
    };
    if workflow.name() != phase.workflow_name() {
        return Err(invalid("workflow name does not match the phase"));
    }
    if !workflow.triggers().manual() || workflow.triggers().push_main() {
        return Err(invalid("only the manual trigger may be enabled"));
    }
    let Some(job) = workflow.only_job() else {
        return Err(invalid("exactly one canary job is required"));
    };
    if !matches!(job.runner(), RunnerSelector::Named(_)) {
        return Err(invalid("an exact named runner is required"));
    }
    if PinnedContainerImage::parse(job.container().image()).is_err() {
        return Err(invalid("container image must be pinned by sha256 digest"));
    }
    if job.timeout_seconds() != RUNNER_PROTOCOL_CANARY_TIMEOUT_SECONDS {
        return Err(invalid("timeout does not match the canary timeout"));
    }
    if job.caches().len() != 1
        || job.caches()[0].as_str() != RUNNER_PROTOCOL_CANARY_CACHE_NAME
        || job.caches()[0].mount_path() != RUNNER_PROTOCOL_CANARY_CACHE_PATH
    {
        return Err(invalid("the runner-protocol cache must be the only cache"));
    }
    if job.steps().len() != 1
        || job.steps()[0].name() != phase.step_name()
        || job.steps()[0].run() != phase.step_command()
    {
        return Err(invalid("the phase-specific step does not match"));
    }
    Ok(())
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum RunnerProtocolCanaryStatus {
    Pending,
    Running,
    Succeeded,
    Failed,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RunnerProtocolCanary {
    generation: CanaryGeneration,
    phase: RunnerProtocolCanaryPhase,
    runner_id: String,
    run_id: String,
    status: RunnerProtocolCanaryStatus,
}

impl RunnerProtocolCanary {
    pub fn new(
        generation: CanaryGeneration,
        phase: RunnerProtocolCanaryPhase,
        runner_id: impl Into<String>,
        run_id: impl Into<String>,
    ) -> Result<Self, DomainError> {
        Self::restore(
            generation,
            phase,
            runner_id,
            run_id,
            RunnerProtocolCanaryStatus::Pending,
        )
    }

    pub fn restore(
        generation: CanaryGeneration,
        phase: RunnerProtocolCanaryPhase,
        runner_id: impl Into<String>,
        run_id: impl Into<String>,
        status: RunnerProtocolCanaryStatus,
    ) -> Result<Self, DomainError> {
        let runner_id = runner_id.into();
        let run_id = run_id.into();
        if runner_id.trim().is_empty() || run_id.trim().is_empty() {
            return Err(DomainError::invalid_input(
                "runner protocol canary requires exact runner and run ids",
            ));
        }
        Ok(Self {
            generation,
            phase,
            runner_id,
            run_id,
            status,
        })
    }

    pub fn generation(&self) -> CanaryGeneration {
        self.generation
    }

    pub fn phase(&self) -> RunnerProtocolCanaryPhase {
        self.phase
    }

    pub fn runner_id(&self) -> &str {
        &self.runner_id
    }

    pub fn run_id(&self) -> &str {
        &self.run_id
    }

    pub fn status(&self) -> RunnerProtocolCanaryStatus {
        self.status
    }

    pub fn start(&mut self) -> Result<bool, DomainError> {
        self.move_to(
            RunnerProtocolCanaryStatus::Pending,
            RunnerProtocolCanaryStatus::Running,
        )
    }

    pub fn succeed(&mut self) -> Result<bool, DomainError> {
        self.move_to(
            RunnerProtocolCanaryStatus::Running,
            RunnerProtocolCanaryStatus::Succeeded,
        )
    }

    pub fn fail(&mut self) -> Result<bool, DomainError> {
        if self.status == RunnerProtocolCanaryStatus::Failed {
            return Ok(false);
        }
        if !matches!(
            self.status,
            RunnerProtocolCanaryStatus::Pending | RunnerProtocolCanaryStatus::Running
        ) {
            return Err(DomainError::conflict(
                "succeeded runner protocol canary cannot fail",
            ));
        }
        self.status = RunnerProtocolCanaryStatus::Failed;
        Ok(true)
    }

    pub fn retire_abandoned(&mut self) -> Result<bool, DomainError> {
        if self.status == RunnerProtocolCanaryStatus::Failed {
            return Ok(false);
        }
        if self.status != RunnerProtocolCanaryStatus::Running {
            return Err(DomainError::conflict(
                "only a running runner protocol canary can be retired as abandoned",
            ));
        }
        self.status = RunnerProtocolCanaryStatus::Failed;
        Ok(true)
    }

    fn move_to(
        &mut self,
        expected: RunnerProtocolCanaryStatus,
        next: RunnerProtocolCanaryStatus,
    ) -> Result<bool, DomainError> {
        if self.status == next {
            return Ok(false);
        }
        if self.status != expected {
            return Err(DomainError::conflict(format!(
                "runner protocol canary cannot transition from {:?} to {next:?}",
                self.status
            )));
        }
        self.status = next;
        Ok(true)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runs::{
        cache::WorkflowCache,
        workflow::{
            ContainerSpec, RunnerSelector, WorkflowJob, WorkflowJobId, WorkflowStep,
            WorkflowTriggers,
        },
    };

    fn canary_workflow(
        phase: RunnerProtocolCanaryPhase,
        caches: Vec<WorkflowCache>,
        steps: Vec<WorkflowStep>,
        image: &str,
        triggers: WorkflowTriggers,
    ) -> CompiledWorkflow {
        CompiledWorkflow::new(
            phase.workflow_name(),
            triggers,
            vec![
                WorkflowJob::new(
                    WorkflowJobId::parse("canary").unwrap(),
                    vec![],
                    RunnerSelector::named("canary-runner").unwrap(),
                    ContainerSpec::new(image).unwrap(),
                    RUNNER_PROTOCOL_CANARY_TIMEOUT_SECONDS,
                    caches,
                    steps,
                )
                .unwrap(),
            ],
        )
        .unwrap()
    }

    fn pinned_image() -> String {
        format!("registry.example/canary@sha256:{}", "a".repeat(64))
    }

    fn canary_cache() -> WorkflowCache {
        WorkflowCache::new(
            RUNNER_PROTOCOL_CANARY_CACHE_NAME,
            RUNNER_PROTOCOL_CANARY_CACHE_PATH,
        )
        .unwrap()
    }

    #[test]
    fn cutover_only_moves_forward_one_state_at_a_time() {
        let mut cutover = RunnerProtocolCutover::new();
        assert_eq!(cutover.state(), RunnerProtocolCutoverState::V7Fenced);
        assert!(
            cutover
                .transition(RunnerProtocolCutoverState::V7Open)
                .unwrap()
        );
        assert!(
            cutover
                .transition(RunnerProtocolCutoverState::V7Fenced)
                .is_err()
        );
        assert!(
            !cutover
                .transition(RunnerProtocolCutoverState::V7Open)
                .unwrap()
        );
    }

    #[test]
    fn state_gates_protocol_operations_without_outer_policy() {
        let fenced = RunnerProtocolCutoverState::V7Fenced;
        assert!(fenced.allows_runner_registration(RUNNER_PROTOCOL_VERSION));
        assert!(!fenced.allows_enqueue());
        assert!(!fenced.allows_claim(RUNNER_PROTOCOL_VERSION));
        assert!(fenced.allows_attempt_operation(RUNNER_PROTOCOL_VERSION));
        assert!(fenced.allows_canary());

        let open = RunnerProtocolCutoverState::V7Open;
        assert!(open.allows_workflow_writes());
        assert!(open.allows_enqueue());
        assert!(open.allows_claim(RUNNER_PROTOCOL_VERSION));
        assert!(!open.allows_claim(RUNNER_PROTOCOL_VERSION - 1));
        assert!(open.allows_attempt_operation(RUNNER_PROTOCOL_VERSION));
        assert!(!open.allows_canary());
    }

    #[test]
    fn canary_binds_one_generation_phase_runner_and_run() {
        assert!(CanaryGeneration::new(0).is_err());
        let generation = CanaryGeneration::new(1).unwrap();
        assert_eq!(generation.next().unwrap().get(), 2);
        assert!(serde_json::from_str::<CanaryGeneration>("0").is_err());

        let mut canary = RunnerProtocolCanary::new(
            generation,
            RunnerProtocolCanaryPhase::ColdWrite,
            "runner-1",
            "run-1",
        )
        .unwrap();
        assert!(canary.start().unwrap());
        assert!(!canary.start().unwrap());
        assert!(canary.succeed().unwrap());
        assert!(canary.fail().is_err());
        assert_eq!(canary.runner_id(), "runner-1");
        assert_eq!(canary.run_id(), "run-1");
    }

    #[test]
    fn pending_or_running_canaries_can_fail_for_replacement() {
        let generation = CanaryGeneration::new(2).unwrap();
        let mut pending = RunnerProtocolCanary::new(
            generation,
            RunnerProtocolCanaryPhase::WarmRead,
            "runner-2",
            "run-2",
        )
        .unwrap();
        assert!(pending.fail().unwrap());
        assert!(!pending.fail().unwrap());
    }

    #[test]
    fn only_an_abandoned_running_canary_can_be_retired() {
        let generation = CanaryGeneration::new(2).unwrap();
        let mut pending = RunnerProtocolCanary::new(
            generation,
            RunnerProtocolCanaryPhase::WarmRead,
            "runner-2",
            "run-2",
        )
        .unwrap();
        assert!(pending.retire_abandoned().is_err());

        pending.start().unwrap();
        assert!(pending.retire_abandoned().unwrap());
        assert!(!pending.retire_abandoned().unwrap());
        assert_eq!(pending.status(), RunnerProtocolCanaryStatus::Failed);
    }

    #[test]
    fn canonical_canary_workflows_are_exact_for_each_phase() {
        for phase in [
            RunnerProtocolCanaryPhase::ColdWrite,
            RunnerProtocolCanaryPhase::WarmRead,
            RunnerProtocolCanaryPhase::Evict,
        ] {
            let workflow = canary_workflow(
                phase,
                vec![canary_cache()],
                vec![WorkflowStep::new(phase.step_name(), phase.step_command()).unwrap()],
                &pinned_image(),
                WorkflowTriggers::new(true, false).unwrap(),
            );
            validate_runner_protocol_canary_workflow(&workflow, phase).unwrap();
            assert_eq!(
                phase.evicts_cache_after_success(),
                phase == RunnerProtocolCanaryPhase::Evict
            );
        }
    }

    #[test]
    fn arbitrary_runs_cannot_satisfy_the_canary_contract() {
        let phase = RunnerProtocolCanaryPhase::ColdWrite;
        let canonical_step = || WorkflowStep::new(phase.step_name(), phase.step_command()).unwrap();
        let build = |caches, steps, image: &str, triggers| {
            canary_workflow(phase, caches, steps, image, triggers)
        };

        for invalid in [
            build(
                vec![],
                vec![canonical_step()],
                &pinned_image(),
                WorkflowTriggers::new(true, false).unwrap(),
            ),
            build(
                vec![canary_cache()],
                vec![
                    canonical_step(),
                    WorkflowStep::new("Arbitrary", "true").unwrap(),
                ],
                &pinned_image(),
                WorkflowTriggers::new(true, false).unwrap(),
            ),
            build(
                vec![canary_cache()],
                vec![canonical_step()],
                "registry.example/canary:latest",
                WorkflowTriggers::new(true, false).unwrap(),
            ),
            build(
                vec![canary_cache()],
                vec![canonical_step()],
                &pinned_image(),
                WorkflowTriggers::new(true, true).unwrap(),
            ),
        ] {
            assert!(validate_runner_protocol_canary_workflow(&invalid, phase).is_err());
        }

        let wrong_step = build(
            vec![canary_cache()],
            vec![WorkflowStep::new(phase.step_name(), "true").unwrap()],
            &pinned_image(),
            WorkflowTriggers::new(true, false).unwrap(),
        );
        assert!(validate_runner_protocol_canary_workflow(&wrong_step, phase).is_err());
    }
}
