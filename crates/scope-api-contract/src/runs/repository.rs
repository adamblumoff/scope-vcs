use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct CreateManualRunQuery {
    pub workflow: String,
    pub git_oid: String,
    pub request_id: String,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
#[cfg_attr(feature = "ts", derive(schemars::JsonSchema, ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(rename_all = "kebab-case"))]
pub enum RunState {
    Queued,
    Dispatching,
    Running,
    Succeeded,
    Failed,
    Canceled,
    Lost,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[cfg_attr(feature = "ts", derive(schemars::JsonSchema, ts_rs::TS))]
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
#[cfg_attr(feature = "ts", derive(schemars::JsonSchema, ts_rs::TS))]
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

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
#[cfg_attr(feature = "ts", derive(schemars::JsonSchema, ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(rename_all = "kebab-case"))]
pub enum RepositoryRunTrigger {
    Manual,
    PushMain,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[cfg_attr(feature = "ts", derive(schemars::JsonSchema, ts_rs::TS))]
pub struct RepositoryRunSummaryResponse {
    pub id: String,
    pub workflow_name: String,
    pub git_oid: String,
    pub trigger: RepositoryRunTrigger,
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
#[cfg_attr(feature = "ts", derive(schemars::JsonSchema, ts_rs::TS))]
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

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
#[cfg_attr(feature = "ts", derive(schemars::JsonSchema, ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(rename_all = "kebab-case"))]
pub enum RepositoryRunAttemptState {
    Dispatching,
    Running,
    Succeeded,
    Failed,
    Canceled,
    Lost,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
#[cfg_attr(feature = "ts", derive(schemars::JsonSchema, ts_rs::TS))]
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

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
#[cfg_attr(feature = "ts", derive(schemars::JsonSchema, ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(tag = "kind", rename_all = "kebab-case"))]
pub enum RepositoryRunTerminalReason {
    StepFailed { step_index: u32, exit_code: i32 },
    TimedOut { step_index: Option<u32> },
    Canceled { step_index: Option<u32> },
    ExecutionLost { step_index: Option<u32> },
    RuntimeSetupFailed { exit_code: i32, message: String },
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
#[cfg_attr(feature = "ts", derive(schemars::JsonSchema, ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(rename_all = "kebab-case"))]
pub enum RepositoryRunCacheColdReason {
    MetadataMissing,
    MetadataInvalid,
    MetadataNotReady,
    VolumeMissing,
    VolumeInvalid,
    BackingDirectoryMissing,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
#[cfg_attr(feature = "ts", derive(schemars::JsonSchema, ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(tag = "kind", rename_all = "kebab-case"))]
pub enum RepositoryRunCachePreparation {
    Exact,
    Compatible,
    Cold {
        reason: RepositoryRunCacheColdReason,
    },
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
#[cfg_attr(feature = "ts", derive(schemars::JsonSchema, ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(rename_all = "kebab-case"))]
pub enum RepositoryRunCacheFinalState {
    Pending,
    Ready,
    Evicted,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[cfg_attr(feature = "ts", derive(schemars::JsonSchema, ts_rs::TS))]
pub struct RepositoryRunCacheObservationResponse {
    pub workflow_path: String,
    pub job_key: String,
    pub identity_digest: String,
    pub preparation: RepositoryRunCachePreparation,
    pub key_ms: u64,
    pub metadata_ms: u64,
    pub size_bytes: u64,
    pub download_verify_ms: u64,
    pub sync_ms: u64,
    pub extraction_ms: u64,
    pub prepare_ms: u64,
    pub final_state: RepositoryRunCacheFinalState,
    pub finalize_ms: Option<u64>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[cfg_attr(feature = "ts", derive(schemars::JsonSchema, ts_rs::TS))]
pub struct RepositoryRunCacheSetupObservationResponse {
    pub authorization_ms: u64,
    pub wall_ms: u64,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[cfg_attr(feature = "ts", derive(schemars::JsonSchema, ts_rs::TS))]
pub struct RepositoryRunCacheResponse {
    pub name: String,
    pub path: String,
    pub observation: Option<RepositoryRunCacheObservationResponse>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[cfg_attr(feature = "ts", derive(schemars::JsonSchema, ts_rs::TS))]
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
#[cfg_attr(feature = "ts", derive(schemars::JsonSchema, ts_rs::TS))]
pub struct RepositoryRunAttemptResponse {
    pub id: String,
    pub number: u32,
    pub external_run_id: Option<String>,
    pub runtime_version: String,
    pub state: RepositoryRunAttemptState,
    pub created_at_unix: u64,
    pub started_at_unix: Option<u64>,
    pub completed_at_unix: Option<u64>,
    pub terminal_reason: Option<RepositoryRunTerminalReason>,
    pub cache_setup: Option<RepositoryRunCacheSetupObservationResponse>,
    pub caches: Vec<RepositoryRunCacheResponse>,
    pub steps: Vec<RepositoryRunStepResponse>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[cfg_attr(feature = "ts", derive(schemars::JsonSchema, ts_rs::TS))]
pub struct RepositoryRunJobResponse {
    pub key: String,
    pub needs: Vec<String>,
    pub pinned_container_image: String,
    pub state: RepositoryRunJobState,
    pub created_at_unix: u64,
    pub started_at_unix: Option<u64>,
    pub updated_at_unix: u64,
    pub completed_at_unix: Option<u64>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[cfg_attr(feature = "ts", derive(schemars::JsonSchema, ts_rs::TS))]
pub struct RepositoryRunJobDetailResponse {
    pub job: RepositoryRunJobResponse,
    pub attempts: Vec<RepositoryRunAttemptResponse>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[cfg_attr(feature = "ts", derive(schemars::JsonSchema, ts_rs::TS))]
pub struct RepositoryRunDetailResponse {
    pub run: RepositoryRunSummaryResponse,
    pub jobs: Vec<RepositoryRunJobDetailResponse>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct PushTriggerCheckResponse {
    pub workflow_path: String,
    pub workflow_name: String,
    pub run: RunResponse,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum PushTriggerEvaluationState {
    Pending,
    Succeeded,
    ConfigurationError,
    Failed,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct PushTriggerEvaluationResponse {
    pub change_version: u64,
    pub head_oid: String,
    pub state: PushTriggerEvaluationState,
    pub message: Option<String>,
    pub checks: Vec<PushTriggerCheckResponse>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct RunLogResponse {
    pub attempt_id: String,
    pub job_key: String,
    pub step_index: u32,
    pub position: u64,
    pub sequence: u64,
    pub text: String,
    pub created_at_unix: u64,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct RunEventsQuery {
    #[serde(default)]
    pub after: u64,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn repository_run_states_keep_the_existing_json() {
        assert_eq!(
            serde_json::to_string(&RunState::Dispatching).unwrap(),
            "\"dispatching\""
        );
        assert_eq!(
            serde_json::to_string(&PushTriggerEvaluationState::ConfigurationError).unwrap(),
            "\"configuration-error\""
        );
        assert_eq!(
            serde_json::to_value(RepositoryRunTerminalReason::StepFailed {
                step_index: 3,
                exit_code: 17,
            })
            .unwrap(),
            serde_json::json!({"kind": "step-failed", "step_index": 3, "exit_code": 17})
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
                key_ms: 5,
                metadata_ms: 7,
                size_bytes: 0,
                download_verify_ms: 0,
                sync_ms: 0,
                extraction_ms: 0,
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
