use scope_domain::runs::{
    cutover::{RunnerProtocolCanaryPhase, RunnerProtocolCanaryStatus, RunnerProtocolCutoverState},
    run::{AttemptState, RunState, StepState},
    runner::{RunnerCapabilities, RunnerMaxConcurrentJobs},
    trigger::PushTriggerEvaluationState,
    workflow::WorkflowJob,
};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct RegisterRunnerRequest {
    pub owner: String,
    pub repo: String,
    pub name: String,
    pub version: String,
    pub protocol_version: u32,
    pub capabilities: RunnerCapabilities,
    pub max_concurrent_jobs: RunnerMaxConcurrentJobs,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct RegisterRunnerResponse {
    pub runner: RunnerResponse,
    pub secret: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct UpgradeRunnerRegistrationRequest {
    pub version: String,
    pub protocol_version: u32,
    pub capabilities: RunnerCapabilities,
    pub max_concurrent_jobs: RunnerMaxConcurrentJobs,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct UpgradeRunnerRegistrationResponse {
    pub runner: RunnerResponse,
    pub secret: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct AdvanceRunnerProtocolCutoverRequest {
    pub state: RunnerProtocolCutoverState,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct CreateRunnerProtocolCanaryRequest {
    pub runner_id: String,
    pub run_id: String,
    pub phase: RunnerProtocolCanaryPhase,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct RunnerProtocolCutoverResponse {
    pub state: RunnerProtocolCutoverState,
    pub generation: u64,
    pub canaries: Vec<RunnerProtocolCanaryResponse>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct RunnerProtocolCanaryResponse {
    pub generation: u64,
    pub phase: RunnerProtocolCanaryPhase,
    pub runner_id: String,
    pub run_id: String,
    pub status: RunnerProtocolCanaryStatus,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct AttachRunnerRepositoryRequest {
    pub name: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct RunnerGrantResponse {
    pub repository_id: String,
    pub name: String,
    pub active: bool,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct RunnerResponse {
    pub id: String,
    pub version: String,
    pub protocol_version: u32,
    pub max_concurrent_jobs: RunnerMaxConcurrentJobs,
    pub enabled: bool,
    pub created_at_unix: u64,
    pub last_seen_at_unix: Option<u64>,
    pub grants: Vec<RunnerGrantResponse>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct CreateManualRunQuery {
    pub workflow: String,
    pub git_oid: String,
    pub request_id: String,
    pub runner: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct RunResponse {
    pub id: String,
    pub repository_id: String,
    pub workflow_name: String,
    pub git_oid: String,
    pub runner_selection: RunRunnerSelection,
    pub state: RunState,
    pub cancellation_requested: bool,
    pub logs_truncated: bool,
    pub created_at_unix: u64,
    pub updated_at_unix: u64,
    pub completed_at_unix: Option<u64>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(tag = "kind", rename_all = "kebab-case"))]
pub enum RunRunnerSelection {
    Any,
    Named { name: String },
    Mixed,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct PushTriggerCheckResponse {
    pub workflow_path: String,
    pub workflow_name: String,
    pub run: RunResponse,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct PushTriggerEvaluationResponse {
    pub change_version: u64,
    pub head_oid: String,
    pub state: PushTriggerEvaluationState,
    pub message: Option<String>,
    pub checks: Vec<PushTriggerCheckResponse>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct RunnerPollResponse {
    pub run: Option<RunnerRunOffer>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct RunnerRunOffer {
    pub run_id: String,
    pub job_key: String,
    pub repository_id: String,
    pub workflow_name: String,
    pub git_oid: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct ClaimRunResponse {
    pub attempt_id: String,
    pub attempt_token: String,
    pub lease_expires_at_unix: u64,
    pub canary_phase: Option<RunnerProtocolCanaryPhase>,
    pub job: RunJobResponse,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct RunJobResponse {
    pub run_id: String,
    pub job_key: String,
    pub repository_id: String,
    pub workflow_path: String,
    pub git_oid: String,
    pub source_digest: String,
    pub pinned_container_image: Option<String>,
    pub definition: WorkflowJob,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct AttemptStatusResponse {
    pub state: AttemptState,
    pub cancellation_requested: bool,
    pub lease_expires_at_unix: u64,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct AttemptRecoveryStatusResponse {
    pub next_log_sequence: u64,
    pub steps: Vec<AttemptStepStatusResponse>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct AttemptStepStatusResponse {
    pub step_index: u32,
    pub state: StepState,
    pub started_at_unix: Option<u64>,
    pub completed_at_unix: Option<u64>,
    pub exit_code: Option<i32>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct AttemptHeartbeatRequest {}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct AttemptCacheFinalizationRequest {
    pub outcome: AttemptCacheFinalizationOutcome,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum AttemptCacheFinalizationOutcome {
    Succeeded,
    Failed,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct PinAttemptContainerImageRequest {
    pub image: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct PinAttemptContainerImageResponse {
    pub image: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct AppendAttemptLogRequest {
    pub step_index: u32,
    pub sequence: u64,
    pub text: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct CompleteAttemptStepRequest {
    pub conclusion: StepConclusionRequest,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum StepConclusionRequest {
    Succeeded,
    Failed { exit_code: i32 },
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct CompleteAttemptRequest {
    pub conclusion: AttemptConclusionRequest,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum AttemptConclusionRequest {
    SetupFailed { exit_code: i32, message: String },
    TimedOut,
    Canceled,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct RunLogResponse {
    pub attempt_id: String,
    pub job_key: String,
    pub step_index: u32,
    pub position: u64,
    pub sequence: u64,
    pub text: String,
    pub created_at_unix: u64,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct RunEventsQuery {
    #[serde(default)]
    pub after: u64,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cutover_contract_uses_stable_kebab_case_domain_values() {
        let response = RunnerProtocolCutoverResponse {
            state: RunnerProtocolCutoverState::V5Fenced,
            generation: 2,
            canaries: vec![RunnerProtocolCanaryResponse {
                generation: 2,
                phase: RunnerProtocolCanaryPhase::WarmRead,
                runner_id: "runner-1".to_string(),
                run_id: "run-1".to_string(),
                status: RunnerProtocolCanaryStatus::Running,
            }],
        };
        let json = serde_json::to_value(&response).unwrap();
        assert_eq!(json["state"], "v5-fenced");
        assert_eq!(json["canaries"][0]["phase"], "warm-read");
        assert_eq!(json["canaries"][0]["status"], "running");
        assert_eq!(
            serde_json::from_value::<RunnerProtocolCutoverResponse>(json).unwrap(),
            response
        );
    }

    #[test]
    fn cache_finalization_and_claim_canary_phase_are_typed() {
        let succeeded = AttemptCacheFinalizationRequest {
            outcome: AttemptCacheFinalizationOutcome::Succeeded,
        };
        let failed = AttemptCacheFinalizationRequest {
            outcome: AttemptCacheFinalizationOutcome::Failed,
        };
        assert_eq!(
            serde_json::to_value(&succeeded).unwrap()["outcome"]["kind"],
            "succeeded"
        );
        let json = serde_json::to_value(&failed).unwrap();
        assert_eq!(json["outcome"]["kind"], "failed");
        assert_eq!(
            serde_json::from_value::<AttemptCacheFinalizationRequest>(json).unwrap(),
            failed
        );

        let claim_without_canary = serde_json::json!({
            "attempt_id": "attempt-1",
            "attempt_token": "secret",
            "lease_expires_at_unix": 10,
            "job": {
                "run_id": "run-1",
                "job_key": "checks",
                "repository_id": "repo-1",
                "workflow_path": "/.scope/runs/test.yml",
                "git_oid": "a",
                "source_digest": "b",
                "pinned_container_image": null,
                "definition": {
                    "id": "checks",
                    "needs": [],
                    "runner": { "kind": "any" },
                    "container": { "image": "image" },
                    "timeout_seconds": 60,
                    "environment": {},
                    "caches": [],
                    "steps": [{ "name": "Test", "run": "true" }]
                }
            }
        });
        assert!(
            serde_json::from_value::<ClaimRunResponse>(claim_without_canary)
                .unwrap()
                .canary_phase
                .is_none()
        );

        let upgrade = UpgradeRunnerRegistrationRequest {
            version: "2.0.0".to_string(),
            protocol_version: 4,
            capabilities: RunnerCapabilities::v1(),
            max_concurrent_jobs: RunnerMaxConcurrentJobs::new(4).unwrap(),
        };
        let json = serde_json::to_value(upgrade).unwrap();
        assert_eq!(json["version"], "2.0.0");
        assert_eq!(json["protocol_version"], 4);
        assert_eq!(json["capabilities"]["operating_system"], "linux");
        assert_eq!(json["max_concurrent_jobs"], 4);
    }
}
