use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum AttemptState {
    Dispatching,
    Running,
    Succeeded,
    Failed,
    Canceled,
    Lost,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum StepState {
    Pending,
    Running,
    Succeeded,
    Failed,
    Canceled,
    Lost,
    Skipped,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum CacheColdReason {
    MetadataMissing,
    MetadataInvalid,
    MetadataNotReady,
    VolumeMissing,
    VolumeInvalid,
    BackingDirectoryMissing,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum CachePreparation {
    Exact,
    Compatible,
    Cold { reason: CacheColdReason },
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum CacheFinalState {
    Pending,
    Ready,
    Evicted,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct WorkflowContainer {
    pub image: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct WorkflowCacheKeyInputs {
    pub files: Vec<String>,
    pub environment: Vec<String>,
    pub source: bool,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct WorkflowCache {
    pub name: String,
    pub path: String,
    pub format: String,
    pub compatibility: WorkflowCacheKeyInputs,
    pub exact: WorkflowCacheKeyInputs,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct WorkflowStep {
    pub name: String,
    pub run: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct WorkflowJob {
    pub id: String,
    pub needs: Vec<String>,
    pub container: WorkflowContainer,
    pub timeout_seconds: u64,
    pub caches: Vec<WorkflowCache>,
    pub environment: BTreeMap<String, String>,
    pub steps: Vec<WorkflowStep>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ClaimRuntimeResponse {
    pub attempt_token: String,
    pub lease_expires_at_unix: u64,
    pub cache_endpoint: String,
    pub cache_grant: String,
    pub job: RunJobResponse,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
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

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct AttemptStatusResponse {
    pub state: AttemptState,
    pub cancellation_requested: bool,
    pub lease_expires_at_unix: u64,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct AttemptHeartbeatResponse {
    pub status: AttemptStatusResponse,
    pub cache_grant: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct AttemptRecoveryStatusResponse {
    pub next_log_sequence: u64,
    pub steps: Vec<AttemptStepStatusResponse>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct AttemptStepStatusResponse {
    pub step_index: u32,
    pub state: StepState,
    pub started_at_unix: Option<u64>,
    pub completed_at_unix: Option<u64>,
    pub exit_code: Option<i32>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct AttemptHeartbeatRequest {
    pub cache_keys: Vec<AttemptCacheKeyMaterial>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct AttemptCacheKeyMaterial {
    pub cache_name: String,
    pub compatibility_inputs_digest: String,
    pub exact_inputs_digest: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct AttemptCacheFinalizationRequest {
    pub outcome: AttemptCacheFinalizationOutcome,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ReportAttemptCachePreparationsRequest {
    pub authorization_ms: u64,
    pub wall_ms: u64,
    pub caches: Vec<AttemptCachePreparationReport>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct AttemptCachePreparationReport {
    pub cache_name: String,
    pub identity_digest: String,
    pub preparation: CachePreparation,
    pub key_ms: u64,
    pub metadata_ms: u64,
    pub size_bytes: u64,
    pub download_verify_ms: u64,
    pub sync_ms: u64,
    pub extraction_ms: u64,
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

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct AppendAttemptLogRequest {
    pub step_index: u32,
    pub sequence: u64,
    pub text: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
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

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn runtime_status_and_cache_reports_keep_the_existing_json() {
        assert_eq!(
            serde_json::to_string(&AttemptState::Running).unwrap(),
            "\"running\""
        );
        assert_eq!(
            serde_json::to_string(&StepState::Skipped).unwrap(),
            "\"skipped\""
        );
        assert_eq!(
            serde_json::to_value(CachePreparation::Cold {
                reason: CacheColdReason::MetadataMissing,
            })
            .unwrap(),
            serde_json::json!({"kind": "cold", "reason": "metadata-missing"})
        );
        assert_eq!(
            serde_json::to_string(&CacheFinalState::Ready).unwrap(),
            "\"ready\""
        );

        let preparation = ReportAttemptCachePreparationsRequest {
            authorization_ms: 4,
            wall_ms: 12,
            caches: vec![AttemptCachePreparationReport {
                cache_name: "cargo".to_string(),
                identity_digest: "a".repeat(64),
                preparation: CachePreparation::Cold {
                    reason: CacheColdReason::MetadataMissing,
                },
                key_ms: 5,
                metadata_ms: 7,
                size_bytes: 0,
                download_verify_ms: 0,
                sync_ms: 0,
                extraction_ms: 0,
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
    fn workflow_job_keeps_the_domain_wire_shape() {
        let image = format!("rust@sha256:{}", "a".repeat(64));
        let json = serde_json::json!({
            "id": "checks",
            "needs": ["build"],
            "container": {"image": image.clone()},
            "timeout_seconds": 1200,
            "caches": [{
                "name": "cargo",
                "path": "/scope/cache/cargo",
                "format": "tar-zstd-v1",
                "compatibility": {"files": ["Cargo.lock"], "environment": [], "source": false},
                "exact": {"files": ["Cargo.lock"], "environment": ["RUSTFLAGS"], "source": true}
            }],
            "environment": {"RUSTFLAGS": "-Dwarnings"},
            "steps": [{"name": "Test", "run": "cargo test"}]
        });
        let job: WorkflowJob = serde_json::from_value(json.clone()).unwrap();
        assert_eq!(serde_json::to_value(&job).unwrap(), json);
        assert_eq!(
            serde_json::to_string(&job).unwrap(),
            format!(
                "{{\"id\":\"checks\",\"needs\":[\"build\"],\"container\":{{\"image\":\"{image}\"}},\"timeout_seconds\":1200,\"caches\":[{{\"name\":\"cargo\",\"path\":\"/scope/cache/cargo\",\"format\":\"tar-zstd-v1\",\"compatibility\":{{\"files\":[\"Cargo.lock\"],\"environment\":[],\"source\":false}},\"exact\":{{\"files\":[\"Cargo.lock\"],\"environment\":[\"RUSTFLAGS\"],\"source\":true}}}}],\"environment\":{{\"RUSTFLAGS\":\"-Dwarnings\"}},\"steps\":[{{\"name\":\"Test\",\"run\":\"cargo test\"}}]}}"
            )
        );
    }

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
}
