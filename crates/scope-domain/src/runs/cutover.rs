use crate::error::DomainError;
use serde::{Deserialize, Deserializer, Serialize, de::Error as _};

use super::{
    run::PinnedContainerImage,
    runner::RUNNER_PROTOCOL_VERSION,
    workflow::{CompiledWorkflow, RunnerSelector},
};

const RUNNER_PROTOCOL_V3: u32 = 3;

pub const RUNNER_PROTOCOL_CANARY_CACHE_NAME: &str = "runner-protocol";
pub const RUNNER_PROTOCOL_CANARY_SENTINEL_PATH: &str =
    "/scope/cache/runner-protocol/v4-canary-sentinel";
pub const RUNNER_PROTOCOL_CANARY_SENTINEL_VALUE: &str = "scope-runner-protocol-v4";
pub const RUNNER_PROTOCOL_CANARY_TIMEOUT_SECONDS: u64 = 5 * 60;
pub const RUNNER_PROTOCOL_CANARY_COLD_WRITE_COMMAND: &str =
    "printf '%s' 'scope-runner-protocol-v4' > /scope/cache/runner-protocol/v4-canary-sentinel";
pub const RUNNER_PROTOCOL_CANARY_READ_COMMAND: &str =
    "test \"$(cat /scope/cache/runner-protocol/v4-canary-sentinel)\" = 'scope-runner-protocol-v4'";

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum RunnerProtocolCutoverState {
    V3Open,
    V3Draining,
    RewriteV4,
    V4Fenced,
    V4Open,
}

impl RunnerProtocolCutoverState {
    pub fn can_transition_to(self, next: Self) -> bool {
        matches!(
            (self, next),
            (Self::V3Open, Self::V3Draining)
                | (Self::V3Draining, Self::RewriteV4)
                | (Self::RewriteV4, Self::V4Fenced)
                | (Self::V4Fenced, Self::V4Open)
        )
    }

    pub fn allows_runner_registration(self, protocol_version: u32) -> bool {
        self.allows_runner_authentication(protocol_version)
    }

    pub fn allows_runner_authentication(self, protocol_version: u32) -> bool {
        matches!(
            (self, protocol_version),
            (Self::V3Open | Self::V3Draining, RUNNER_PROTOCOL_V3)
                | (Self::V4Fenced | Self::V4Open, RUNNER_PROTOCOL_VERSION)
        )
    }

    pub fn allows_workflow_writes(self) -> bool {
        matches!(self, Self::V3Open | Self::V4Open)
    }

    pub fn allows_enqueue(self) -> bool {
        self.allows_workflow_writes()
    }

    pub fn allows_claim(self, protocol_version: u32) -> bool {
        matches!(
            (self, protocol_version),
            (Self::V3Open, RUNNER_PROTOCOL_V3) | (Self::V4Open, RUNNER_PROTOCOL_VERSION)
        )
    }

    pub fn allows_attempt_operation(
        self,
        operation: RunnerProtocolAttemptOperation,
        protocol_version: u32,
    ) -> bool {
        match (self, protocol_version) {
            (Self::V3Open, RUNNER_PROTOCOL_V3)
            | (Self::V4Fenced | Self::V4Open, RUNNER_PROTOCOL_VERSION) => true,
            (Self::V3Draining, RUNNER_PROTOCOL_V3) => matches!(
                operation,
                RunnerProtocolAttemptOperation::Heartbeat
                    | RunnerProtocolAttemptOperation::Conclusion
                    | RunnerProtocolAttemptOperation::Recovery
            ),
            _ => false,
        }
    }

    pub fn allows_canary(self) -> bool {
        self == Self::V4Fenced
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RunnerProtocolAttemptOperation {
    Heartbeat,
    Conclusion,
    Recovery,
    Execution,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RunnerProtocolCutover {
    state: RunnerProtocolCutoverState,
}

impl RunnerProtocolCutover {
    pub fn new() -> Self {
        Self {
            state: RunnerProtocolCutoverState::V3Open,
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
            Self::ColdWrite => "Runner protocol V4 cold-write canary",
            Self::WarmRead => "Runner protocol V4 warm-read canary",
            Self::Evict => "Runner protocol V4 eviction canary",
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
    if !matches!(workflow.runner(), RunnerSelector::Named(_)) {
        return Err(invalid("an exact named runner is required"));
    }
    if PinnedContainerImage::parse(workflow.container().image()).is_err() {
        return Err(invalid("container image must be pinned by sha256 digest"));
    }
    if workflow.timeout_seconds() != RUNNER_PROTOCOL_CANARY_TIMEOUT_SECONDS {
        return Err(invalid("timeout does not match the canary timeout"));
    }
    if workflow.caches().len() != 1
        || workflow.caches()[0].as_str() != RUNNER_PROTOCOL_CANARY_CACHE_NAME
    {
        return Err(invalid("the runner-protocol cache must be the only cache"));
    }
    if workflow.steps().len() != 1
        || workflow.steps()[0].name() != phase.step_name()
        || workflow.steps()[0].run() != phase.step_command()
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
        workflow::{ContainerSpec, RunnerSelector, WorkflowStep, WorkflowTriggers},
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
            RunnerSelector::named("canary-runner").unwrap(),
            ContainerSpec::new(image).unwrap(),
            RUNNER_PROTOCOL_CANARY_TIMEOUT_SECONDS,
            caches,
            steps,
        )
        .unwrap()
    }

    fn pinned_image() -> String {
        format!("registry.example/canary@sha256:{}", "a".repeat(64))
    }

    #[test]
    fn cutover_only_moves_forward_one_state_at_a_time() {
        let mut cutover = RunnerProtocolCutover::new();
        assert_eq!(cutover.state(), RunnerProtocolCutoverState::V3Open);
        assert!(
            cutover
                .transition(RunnerProtocolCutoverState::V3Draining)
                .unwrap()
        );
        assert!(
            cutover
                .transition(RunnerProtocolCutoverState::V4Fenced)
                .is_err()
        );
        assert!(
            cutover
                .transition(RunnerProtocolCutoverState::V3Open)
                .is_err()
        );
        assert!(
            !cutover
                .transition(RunnerProtocolCutoverState::V3Draining)
                .unwrap()
        );
    }

    #[test]
    fn state_gates_protocol_operations_without_outer_policy() {
        use RunnerProtocolAttemptOperation::{Conclusion, Execution, Heartbeat, Recovery};

        let draining = RunnerProtocolCutoverState::V3Draining;
        assert!(draining.allows_runner_authentication(RUNNER_PROTOCOL_V3));
        assert!(!draining.allows_enqueue());
        assert!(!draining.allows_claim(RUNNER_PROTOCOL_V3));
        assert!(draining.allows_attempt_operation(Heartbeat, RUNNER_PROTOCOL_V3));
        assert!(draining.allows_attempt_operation(Conclusion, RUNNER_PROTOCOL_V3));
        assert!(draining.allows_attempt_operation(Recovery, RUNNER_PROTOCOL_V3));
        assert!(!draining.allows_attempt_operation(Execution, RUNNER_PROTOCOL_V3));

        let fenced = RunnerProtocolCutoverState::V4Fenced;
        assert!(fenced.allows_runner_registration(RUNNER_PROTOCOL_VERSION));
        assert!(!fenced.allows_enqueue());
        assert!(!fenced.allows_claim(RUNNER_PROTOCOL_VERSION));
        assert!(fenced.allows_canary());

        let open = RunnerProtocolCutoverState::V4Open;
        assert!(open.allows_workflow_writes());
        assert!(open.allows_enqueue());
        assert!(open.allows_claim(RUNNER_PROTOCOL_VERSION));
        assert!(!open.allows_claim(RUNNER_PROTOCOL_V3));
        assert!(!open.allows_canary());

        let rewrite = RunnerProtocolCutoverState::RewriteV4;
        assert!(!rewrite.allows_runner_authentication(RUNNER_PROTOCOL_VERSION));
        assert!(!rewrite.allows_attempt_operation(Conclusion, RUNNER_PROTOCOL_V3));
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
    fn canonical_canary_workflows_are_exact_for_each_phase() {
        let cache = || WorkflowCache::parse(RUNNER_PROTOCOL_CANARY_CACHE_NAME).unwrap();
        for phase in [
            RunnerProtocolCanaryPhase::ColdWrite,
            RunnerProtocolCanaryPhase::WarmRead,
            RunnerProtocolCanaryPhase::Evict,
        ] {
            let workflow = canary_workflow(
                phase,
                vec![cache()],
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
                vec![WorkflowCache::parse(RUNNER_PROTOCOL_CANARY_CACHE_NAME).unwrap()],
                vec![
                    canonical_step(),
                    WorkflowStep::new("Arbitrary", "true").unwrap(),
                ],
                &pinned_image(),
                WorkflowTriggers::new(true, false).unwrap(),
            ),
            build(
                vec![WorkflowCache::parse(RUNNER_PROTOCOL_CANARY_CACHE_NAME).unwrap()],
                vec![canonical_step()],
                "registry.example/canary:latest",
                WorkflowTriggers::new(true, false).unwrap(),
            ),
            build(
                vec![WorkflowCache::parse(RUNNER_PROTOCOL_CANARY_CACHE_NAME).unwrap()],
                vec![canonical_step()],
                &pinned_image(),
                WorkflowTriggers::new(true, true).unwrap(),
            ),
        ] {
            assert!(validate_runner_protocol_canary_workflow(&invalid, phase).is_err());
        }

        let wrong_step = build(
            vec![WorkflowCache::parse(RUNNER_PROTOCOL_CANARY_CACHE_NAME).unwrap()],
            vec![WorkflowStep::new(phase.step_name(), "true").unwrap()],
            &pinned_image(),
            WorkflowTriggers::new(true, false).unwrap(),
        );
        assert!(validate_runner_protocol_canary_workflow(&wrong_step, phase).is_err());
    }
}
