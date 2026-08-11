use scope_domain::runs::{
    cache::{CacheColdReason, CacheFinalState, CachePreparation},
    cutover::{RunnerProtocolCanaryPhase, RunnerProtocolCanaryStatus, RunnerProtocolCutoverState},
    resources::JobResources,
    run::{AttemptState, AttemptTerminalReason, RunJobState, RunState, StepState},
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
    pub enabled_runner_count: u64,
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

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(rename_all = "kebab-case"))]
pub enum RepositoryRunState {
    Queued,
    Leased,
    Running,
    Succeeded,
    Failed,
    Canceled,
    Lost,
}

impl From<RunState> for RepositoryRunState {
    fn from(state: RunState) -> Self {
        match state {
            RunState::Queued => Self::Queued,
            RunState::Leased => Self::Leased,
            RunState::Running => Self::Running,
            RunState::Succeeded => Self::Succeeded,
            RunState::Failed => Self::Failed,
            RunState::Canceled => Self::Canceled,
            RunState::Lost => Self::Lost,
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct RepositoryRunSummaryResponse {
    pub id: String,
    pub workflow_name: String,
    pub git_oid: String,
    pub runner_selection: RunRunnerSelection,
    pub state: RepositoryRunState,
    pub cancellation_requested: bool,
    pub created_at_unix: u64,
    pub updated_at_unix: u64,
    pub completed_at_unix: Option<u64>,
    pub can_cancel: bool,
    pub can_retry: bool,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(rename_all = "kebab-case"))]
pub enum RepositoryRunJobState {
    Blocked,
    Queued,
    Leased,
    Running,
    Succeeded,
    Failed,
    Skipped,
    Canceled,
    Lost,
}

impl From<RunJobState> for RepositoryRunJobState {
    fn from(state: RunJobState) -> Self {
        match state {
            RunJobState::Blocked => Self::Blocked,
            RunJobState::Queued => Self::Queued,
            RunJobState::Leased => Self::Leased,
            RunJobState::Running => Self::Running,
            RunJobState::Succeeded => Self::Succeeded,
            RunJobState::Failed => Self::Failed,
            RunJobState::Skipped => Self::Skipped,
            RunJobState::Canceled => Self::Canceled,
            RunJobState::Lost => Self::Lost,
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(rename_all = "kebab-case"))]
pub enum RepositoryRunAttemptState {
    Leased,
    Running,
    Succeeded,
    Failed,
    Canceled,
    Lost,
}

impl From<AttemptState> for RepositoryRunAttemptState {
    fn from(state: AttemptState) -> Self {
        match state {
            AttemptState::Leased => Self::Leased,
            AttemptState::Running => Self::Running,
            AttemptState::Succeeded => Self::Succeeded,
            AttemptState::Failed => Self::Failed,
            AttemptState::Canceled => Self::Canceled,
            AttemptState::Lost => Self::Lost,
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(rename_all = "kebab-case"))]
pub enum RepositoryRunStepState {
    Pending,
    Running,
    Succeeded,
    Failed,
    Canceled,
    Lost,
    Skipped,
}

impl From<StepState> for RepositoryRunStepState {
    fn from(state: StepState) -> Self {
        match state {
            StepState::Pending => Self::Pending,
            StepState::Running => Self::Running,
            StepState::Succeeded => Self::Succeeded,
            StepState::Failed => Self::Failed,
            StepState::Canceled => Self::Canceled,
            StepState::Lost => Self::Lost,
            StepState::Skipped => Self::Skipped,
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(tag = "kind", rename_all = "kebab-case"))]
pub enum RepositoryRunTerminalReason {
    StepFailed { step_index: u32, exit_code: i32 },
    TimedOut { step_index: Option<u32> },
    Canceled { step_index: Option<u32> },
    RunnerLost { step_index: Option<u32> },
    RunnerSetupFailed { exit_code: i32, message: String },
}

impl From<AttemptTerminalReason> for RepositoryRunTerminalReason {
    fn from(reason: AttemptTerminalReason) -> Self {
        match reason {
            AttemptTerminalReason::StepFailed {
                step_index,
                exit_code,
            } => Self::StepFailed {
                step_index,
                exit_code,
            },
            AttemptTerminalReason::TimedOut { step_index } => Self::TimedOut { step_index },
            AttemptTerminalReason::Canceled { step_index } => Self::Canceled { step_index },
            AttemptTerminalReason::RunnerLost { step_index } => Self::RunnerLost { step_index },
            AttemptTerminalReason::RunnerSetupFailed { exit_code, message } => {
                Self::RunnerSetupFailed { exit_code, message }
            }
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(rename_all = "kebab-case"))]
pub enum RepositoryRunCacheColdReason {
    MetadataMissing,
    MetadataInvalid,
    MetadataNotReady,
    VolumeMissing,
    VolumeInvalid,
    BackingDirectoryMissing,
}

impl From<CacheColdReason> for RepositoryRunCacheColdReason {
    fn from(reason: CacheColdReason) -> Self {
        match reason {
            CacheColdReason::MetadataMissing => Self::MetadataMissing,
            CacheColdReason::MetadataInvalid => Self::MetadataInvalid,
            CacheColdReason::MetadataNotReady => Self::MetadataNotReady,
            CacheColdReason::VolumeMissing => Self::VolumeMissing,
            CacheColdReason::VolumeInvalid => Self::VolumeInvalid,
            CacheColdReason::BackingDirectoryMissing => Self::BackingDirectoryMissing,
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(tag = "kind", rename_all = "kebab-case"))]
pub enum RepositoryRunCachePreparation {
    Warm,
    Cold {
        reason: RepositoryRunCacheColdReason,
    },
}

impl From<CachePreparation> for RepositoryRunCachePreparation {
    fn from(preparation: CachePreparation) -> Self {
        match preparation {
            CachePreparation::Warm => Self::Warm,
            CachePreparation::Cold { reason } => Self::Cold {
                reason: reason.into(),
            },
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(rename_all = "kebab-case"))]
pub enum RepositoryRunCacheFinalState {
    Pending,
    Ready,
    Evicted,
}

impl From<CacheFinalState> for RepositoryRunCacheFinalState {
    fn from(state: CacheFinalState) -> Self {
        match state {
            CacheFinalState::Pending => Self::Pending,
            CacheFinalState::Ready => Self::Ready,
            CacheFinalState::Evicted => Self::Evicted,
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct RepositoryRunCacheObservationResponse {
    pub workflow_path: String,
    pub job_key: String,
    pub identity_digest: String,
    pub preparation: RepositoryRunCachePreparation,
    pub prepare_ms: u64,
    pub final_state: RepositoryRunCacheFinalState,
    pub finalize_ms: Option<u64>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct RepositoryRunCacheResponse {
    pub name: String,
    pub path: String,
    pub observation: Option<RepositoryRunCacheObservationResponse>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct RepositoryRunStepResponse {
    pub index: u32,
    pub name: String,
    pub command: String,
    pub state: RepositoryRunStepState,
    pub started_at_unix: Option<u64>,
    pub completed_at_unix: Option<u64>,
    pub exit_code: Option<i32>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct RepositoryRunAttemptResponse {
    pub id: String,
    pub runner_id: String,
    pub runner_name: String,
    pub state: RepositoryRunAttemptState,
    pub created_at_unix: u64,
    pub started_at_unix: Option<u64>,
    pub completed_at_unix: Option<u64>,
    pub terminal_reason: Option<RepositoryRunTerminalReason>,
    pub caches: Vec<RepositoryRunCacheResponse>,
    pub steps: Vec<RepositoryRunStepResponse>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct RepositoryRunJobResponse {
    pub key: String,
    pub needs: Vec<String>,
    pub desired_runner: Option<String>,
    pub pinned_container_image: Option<String>,
    pub state: RepositoryRunJobState,
    pub created_at_unix: u64,
    pub updated_at_unix: u64,
    pub completed_at_unix: Option<u64>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct RepositoryRunJobDetailResponse {
    pub job: RepositoryRunJobResponse,
    pub attempts: Vec<RepositoryRunAttemptResponse>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct RepositoryRunDetailResponse {
    pub run: RepositoryRunSummaryResponse,
    pub jobs: Vec<RepositoryRunJobDetailResponse>,
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
pub struct RunnerPollRequest {
    pub available_resources: JobResources,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct RunnerPollResponse {
    pub claim: Option<ClaimRunResponse>,
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
pub struct ReportAttemptCachePreparationsRequest {
    pub caches: Vec<AttemptCachePreparationReport>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct AttemptCachePreparationReport {
    pub cache_name: String,
    pub identity_digest: String,
    pub preparation: CachePreparation,
    pub prepare_ms: u64,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ReportAttemptCacheFinalizationsRequest {
    pub caches: Vec<AttemptCacheFinalizationReport>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct AttemptCacheFinalizationReport {
    pub identity_digest: String,
    pub final_state: CacheFinalState,
    pub finalize_ms: u64,
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
    pub logs_truncated: bool,
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
    pub logs_truncated: bool,
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
    use scope_domain::runs::cache::{CacheColdReason, CacheFinalState, CachePreparation};

    #[test]
    fn cutover_contract_uses_stable_kebab_case_domain_values() {
        let response = RunnerProtocolCutoverResponse {
            state: RunnerProtocolCutoverState::V8Fenced,
            generation: 2,
            enabled_runner_count: 1,
            canaries: vec![RunnerProtocolCanaryResponse {
                generation: 2,
                phase: RunnerProtocolCanaryPhase::WarmRead,
                runner_id: "runner-1".to_string(),
                run_id: "run-1".to_string(),
                status: RunnerProtocolCanaryStatus::Running,
            }],
        };
        let json = serde_json::to_value(&response).unwrap();
        assert_eq!(json["state"], "v8-fenced");
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
                    "resources": { "cpu_millis": 1000, "memory_bytes": 1073741824_u64 },
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

    #[test]
    fn cache_observation_contract_uses_constrained_domain_values() {
        let preparation = ReportAttemptCachePreparationsRequest {
            caches: vec![AttemptCachePreparationReport {
                cache_name: "cargo".to_string(),
                identity_digest: "a".repeat(64),
                preparation: CachePreparation::Cold {
                    reason: CacheColdReason::MetadataMissing,
                },
                prepare_ms: 12,
            }],
        };
        let finalization = ReportAttemptCacheFinalizationsRequest {
            caches: vec![AttemptCacheFinalizationReport {
                identity_digest: "a".repeat(64),
                final_state: CacheFinalState::Ready,
                finalize_ms: 8,
            }],
        };

        let preparation_json = serde_json::to_value(preparation).unwrap();
        assert_eq!(preparation_json["caches"][0]["preparation"]["kind"], "cold");
        assert_eq!(
            preparation_json["caches"][0]["preparation"]["reason"],
            "metadata-missing"
        );
        assert_eq!(
            serde_json::to_value(finalization).unwrap()["caches"][0]["final_state"],
            "ready"
        );
    }

    #[test]
    fn repository_cache_contract_keeps_missing_reports_distinct_from_cold() {
        let cold = RepositoryRunCacheResponse {
            name: "cargo".to_string(),
            path: "/scope/cache/cargo".to_string(),
            observation: Some(RepositoryRunCacheObservationResponse {
                workflow_path: "/.scope/runs/checks.yml".to_string(),
                job_key: "backend".to_string(),
                identity_digest: "a".repeat(64),
                preparation: RepositoryRunCachePreparation::Cold {
                    reason: RepositoryRunCacheColdReason::MetadataMissing,
                },
                prepare_ms: 12,
                final_state: RepositoryRunCacheFinalState::Ready,
                finalize_ms: Some(8),
            }),
        };
        let unavailable = RepositoryRunCacheResponse {
            name: "target".to_string(),
            path: "/workspace/target".to_string(),
            observation: None,
        };

        let cold_json = serde_json::to_value(&cold).unwrap();
        assert_eq!(cold_json["observation"]["preparation"]["kind"], "cold");
        assert_eq!(
            cold_json["observation"]["preparation"]["reason"],
            "metadata-missing"
        );
        assert_eq!(
            serde_json::to_value(&unavailable).unwrap()["observation"],
            serde_json::Value::Null
        );
        assert_eq!(
            serde_json::from_value::<RepositoryRunCacheResponse>(cold_json).unwrap(),
            cold
        );
    }
}
