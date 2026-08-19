use scope_domain::runs::{
    cache::{CacheColdReason, CacheFinalState, CachePreparation},
    run::{
        AttemptState, AttemptTerminalReason, ExecutionProvider, RunJobState, RunState, StepState,
    },
    trigger::PushTriggerEvaluationState,
    workflow::WorkflowJob,
};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct CreateManualRunQuery {
    pub workflow: String,
    pub git_oid: String,
    pub request_id: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct RunResponse {
    pub id: String,
    pub repository_id: String,
    pub workflow_name: String,
    pub git_oid: String,
    pub state: RunState,
    pub cancellation_requested: bool,
    pub logs_truncated: bool,
    pub created_at_unix: u64,
    pub updated_at_unix: u64,
    pub completed_at_unix: Option<u64>,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(rename_all = "kebab-case"))]
pub enum RepositoryRunState {
    Queued,
    Dispatching,
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
            RunState::Dispatching => Self::Dispatching,
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
    Dispatching,
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
            RunJobState::Dispatching => Self::Dispatching,
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
    Dispatching,
    Running,
    Succeeded,
    Failed,
    Canceled,
    Lost,
}

impl From<AttemptState> for RepositoryRunAttemptState {
    fn from(state: AttemptState) -> Self {
        match state {
            AttemptState::Dispatching => Self::Dispatching,
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
    ExecutionLost { step_index: Option<u32> },
    RuntimeSetupFailed { exit_code: i32, message: String },
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
            AttemptTerminalReason::ExecutionLost { step_index } => {
                Self::ExecutionLost { step_index }
            }
            AttemptTerminalReason::RuntimeSetupFailed { exit_code, message } => {
                Self::RuntimeSetupFailed { exit_code, message }
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
    pub execution_provider: RepositoryExecutionProvider,
    pub external_run_id: Option<String>,
    pub runtime_version: String,
    pub state: RepositoryRunAttemptState,
    pub created_at_unix: u64,
    pub started_at_unix: Option<u64>,
    pub completed_at_unix: Option<u64>,
    pub terminal_reason: Option<RepositoryRunTerminalReason>,
    pub caches: Vec<RepositoryRunCacheResponse>,
    pub steps: Vec<RepositoryRunStepResponse>,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(rename_all = "kebab-case"))]
pub enum RepositoryExecutionProvider {
    Northflank,
}

impl From<ExecutionProvider> for RepositoryExecutionProvider {
    fn from(provider: ExecutionProvider) -> Self {
        match provider {
            ExecutionProvider::Northflank => Self::Northflank,
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct RepositoryRunJobResponse {
    pub key: String,
    pub needs: Vec<String>,
    pub pinned_container_image: String,
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
pub struct ClaimRuntimeResponse {
    pub attempt_token: String,
    pub lease_expires_at_unix: u64,
    pub cache_endpoint: String,
    pub cache_grant: String,
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
    pub pinned_container_image: String,
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
    Succeeded,
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
    fn cache_finalization_is_typed() {
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
