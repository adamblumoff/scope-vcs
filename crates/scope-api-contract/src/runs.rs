use scope_domain::runs::{
    run::{AttemptState, RunState},
    runner::RunnerCapabilities,
    trigger::PushTriggerEvaluationState,
    workflow::CompiledWorkflow,
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
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct RegisterRunnerResponse {
    pub runner: RunnerResponse,
    pub secret: String,
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
    pub desired_runner: Option<String>,
    pub state: RunState,
    pub cancellation_requested: bool,
    pub logs_truncated: bool,
    pub attempt_number: u32,
    pub created_at_unix: u64,
    pub updated_at_unix: u64,
    pub completed_at_unix: Option<u64>,
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
    pub repository_id: String,
    pub workflow_name: String,
    pub git_oid: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct ClaimRunResponse {
    pub attempt_id: String,
    pub attempt_token: String,
    pub lease_expires_at_unix: u64,
    pub job: RunJobResponse,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct RunJobResponse {
    pub run_id: String,
    pub repository_id: String,
    pub git_oid: String,
    pub source_digest: String,
    pub pinned_container_image: Option<String>,
    pub workflow: CompiledWorkflow,
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
    pub log_bytes: u64,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct AttemptHeartbeatRequest {}

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
    pub sequence: u64,
    pub text: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct CompleteAttemptRequest {
    pub conclusion: AttemptConclusionRequest,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum AttemptConclusionRequest {
    Succeeded,
    Failed { exit_code: i32 },
    Canceled,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct RunLogResponse {
    pub attempt_id: String,
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
