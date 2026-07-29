use super::{
    runner::{Runner, RunnerGrant, required, validate_sha256_hash},
    workflow::{RunnerSelector, WorkflowIdentity, WorkflowRevision},
};
use crate::{
    content_ref::ContentRef, error::DomainError, projection::ProjectionViewKey, store::SourceBlob,
};
use serde::{Deserialize, Serialize};

pub const MAX_RUN_LOG_CHUNK_BYTES: usize = 64 * 1024;
pub const MAX_RUN_LOG_BYTES_PER_ATTEMPT: u64 = 10 * 1024 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum RunTrigger {
    Manual,
    PushMain,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum RunState {
    Queued,
    Leased,
    Running,
    Succeeded,
    Failed,
    Canceled,
    Lost,
}

impl RunState {
    pub fn is_terminal(self) -> bool {
        matches!(
            self,
            Self::Succeeded | Self::Failed | Self::Canceled | Self::Lost
        )
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum AttemptState {
    Leased,
    Running,
    Succeeded,
    Failed,
    Canceled,
    Lost,
}

impl AttemptState {
    pub fn is_terminal(self) -> bool {
        matches!(
            self,
            Self::Succeeded | Self::Failed | Self::Canceled | Self::Lost
        )
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AttemptConclusion {
    Succeeded,
    Failed { exit_code: i32 },
    Canceled,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RunLogChunk {
    pub attempt_id: String,
    pub sequence: u64,
    pub text: String,
    pub created_at_unix: u64,
}

impl RunLogChunk {
    pub fn new(
        attempt_id: impl Into<String>,
        sequence: u64,
        text: impl Into<String>,
        created_at_unix: u64,
    ) -> Result<Self, DomainError> {
        let attempt_id = required("run log attempt id", attempt_id.into())?;
        if sequence == 0 {
            return Err(DomainError::invalid_input(
                "run log sequence must be positive",
            ));
        }
        let text = text.into();
        if text.is_empty() {
            return Err(DomainError::invalid_input("run log text is required"));
        }
        if text.len() > MAX_RUN_LOG_CHUNK_BYTES {
            return Err(DomainError::invalid_input(format!(
                "run log chunk cannot exceed {MAX_RUN_LOG_CHUNK_BYTES} bytes"
            )));
        }
        Ok(Self {
            attempt_id,
            sequence,
            text,
            created_at_unix,
        })
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum RunSource {
    EphemeralGitBundle {
        object: SourceBlob,
    },
    AcceptedRevision {
        change_version: u64,
        manifest: SourceBlob,
        snapshot: SourceBlob,
        audience: ProjectionViewKey,
    },
}

impl RunSource {
    pub fn ephemeral_git_bundle(object: SourceBlob) -> Result<Self, DomainError> {
        validate_source_blob(&object, "run source bundle")?;
        if !matches!(object.content_ref, ContentRef::GitBundleSha256(_)) {
            return Err(DomainError::invalid_input(
                "ephemeral run source must be a Git bundle",
            ));
        }
        Ok(Self::EphemeralGitBundle { object })
    }

    pub fn accepted_revision(
        change_version: u64,
        manifest: SourceBlob,
        snapshot: SourceBlob,
        audience: ProjectionViewKey,
    ) -> Result<Self, DomainError> {
        if change_version == 0 {
            return Err(DomainError::invalid_input(
                "accepted run source change version must be positive",
            ));
        }
        validate_source_blob(&manifest, "run source manifest")?;
        if !matches!(manifest.content_ref, ContentRef::GitManifestSha256(_)) {
            return Err(DomainError::invalid_input(
                "accepted run source must use a Git manifest",
            ));
        }
        validate_source_blob(&snapshot, "accepted run source snapshot")?;
        if !matches!(snapshot.content_ref, ContentRef::GitBundleSha256(_)) {
            return Err(DomainError::invalid_input(
                "accepted run source snapshot must be a Git bundle",
            ));
        }
        if snapshot.git_oid != manifest.git_oid {
            return Err(DomainError::invalid_input(
                "accepted run source snapshot and manifest heads do not match",
            ));
        }
        Ok(Self::AcceptedRevision {
            change_version,
            manifest,
            snapshot,
            audience,
        })
    }

    pub fn snapshot(&self) -> &SourceBlob {
        match self {
            Self::EphemeralGitBundle { object } => object,
            Self::AcceptedRevision { snapshot, .. } => snapshot,
        }
    }

    pub fn retained_objects(&self) -> Vec<&SourceBlob> {
        match self {
            Self::EphemeralGitBundle { object } => vec![object],
            Self::AcceptedRevision {
                manifest, snapshot, ..
            } => vec![manifest, snapshot],
        }
    }

    pub fn digest(&self) -> &str {
        &self.snapshot().sha256
    }

    pub fn git_oid(&self) -> &str {
        &self.snapshot().git_oid
    }

    pub fn is_private_only(&self) -> bool {
        matches!(
            self,
            Self::EphemeralGitBundle { .. }
                | Self::AcceptedRevision {
                    audience: ProjectionViewKey::Private,
                    ..
                }
        )
    }
}

fn validate_source_blob(blob: &SourceBlob, label: &str) -> Result<(), DomainError> {
    validate_sha256_hash(&format!("{label} digest"), &blob.sha256)?;
    if blob.content_ref.sha256() != Some(blob.sha256.as_str()) {
        return Err(DomainError::invalid_input(format!(
            "{label} content reference does not match its digest"
        )));
    }
    validate_git_oid(&format!("{label} Git OID"), &blob.git_oid)
}

fn validate_git_oid(label: &str, git_oid: &str) -> Result<(), DomainError> {
    if git_oid.len() != 40 || !git_oid.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(DomainError::invalid_input(format!(
            "{label} must be a SHA-1 hex digest"
        )));
    }
    Ok(())
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PinnedContainerImage(String);

impl PinnedContainerImage {
    pub fn parse(image: impl Into<String>) -> Result<Self, DomainError> {
        let image = image.into();
        let Some((repository, digest)) = image.rsplit_once("@sha256:") else {
            return Err(DomainError::invalid_input(
                "pinned container image must end in an immutable sha256 digest",
            ));
        };
        if repository.is_empty()
            || repository.contains('@')
            || image.chars().any(char::is_whitespace)
            || digest.len() != 64
            || !digest.bytes().all(|byte| byte.is_ascii_hexdigit())
        {
            return Err(DomainError::invalid_input(
                "pinned container image is invalid",
            ));
        }
        Ok(Self(format!(
            "{repository}@sha256:{}",
            digest.to_ascii_lowercase()
        )))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Run {
    pub id: String,
    pub idempotency_key: String,
    pub workflow: WorkflowIdentity,
    pub workflow_revision_digest: String,
    pub trigger: RunTrigger,
    pub requested_by_user_id: Option<String>,
    pub source: RunSource,
    pub pinned_container_image: Option<PinnedContainerImage>,
    pub desired_runner: RunnerSelector,
    pub state: RunState,
    pub cancellation_requested: bool,
    pub last_attempt_number: u32,
    pub current_attempt_id: Option<String>,
    pub created_at_unix: u64,
    pub updated_at_unix: u64,
    pub completed_at_unix: Option<u64>,
}

impl Run {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        id: impl Into<String>,
        idempotency_key: impl Into<String>,
        workflow: WorkflowIdentity,
        workflow_revision_digest: impl Into<String>,
        trigger: RunTrigger,
        requested_by_user_id: Option<String>,
        source: RunSource,
        desired_runner: RunnerSelector,
        now_unix: u64,
    ) -> Result<Self, DomainError> {
        let id = required("run id", id.into())?;
        let idempotency_key = required("run idempotency key", idempotency_key.into())?;
        let workflow_revision_digest = workflow_revision_digest.into();
        validate_sha256_hash("workflow revision digest", &workflow_revision_digest)?;
        desired_runner
            .validate()
            .map_err(|error| DomainError::invalid_input(error.to_string()))?;
        if requested_by_user_id
            .as_deref()
            .is_some_and(|id| id.trim().is_empty())
        {
            return Err(DomainError::invalid_input(
                "run requester user id cannot be empty",
            ));
        }
        if trigger == RunTrigger::Manual
            && requested_by_user_id
                .as_deref()
                .is_none_or(|id| id.trim().is_empty())
        {
            return Err(DomainError::invalid_input(
                "manual run requester user id is required",
            ));
        }
        Ok(Self {
            id,
            idempotency_key,
            workflow,
            workflow_revision_digest,
            trigger,
            requested_by_user_id,
            source,
            pinned_container_image: None,
            desired_runner,
            state: RunState::Queued,
            cancellation_requested: false,
            last_attempt_number: 0,
            current_attempt_id: None,
            created_at_unix: now_unix,
            updated_at_unix: now_unix,
            completed_at_unix: None,
        })
    }

    #[allow(clippy::too_many_arguments)]
    pub fn restore(
        id: impl Into<String>,
        idempotency_key: impl Into<String>,
        workflow: WorkflowIdentity,
        workflow_revision_digest: impl Into<String>,
        trigger: RunTrigger,
        requested_by_user_id: Option<String>,
        source: RunSource,
        pinned_container_image: Option<PinnedContainerImage>,
        desired_runner: RunnerSelector,
        state: RunState,
        cancellation_requested: bool,
        last_attempt_number: u32,
        current_attempt_id: Option<String>,
        created_at_unix: u64,
        updated_at_unix: u64,
        completed_at_unix: Option<u64>,
    ) -> Result<Self, DomainError> {
        let mut run = Self::new(
            id,
            idempotency_key,
            workflow,
            workflow_revision_digest,
            trigger,
            requested_by_user_id,
            source,
            desired_runner,
            created_at_unix,
        )?;
        run.pinned_container_image = pinned_container_image;
        run.state = state;
        run.cancellation_requested = cancellation_requested;
        run.last_attempt_number = last_attempt_number;
        run.current_attempt_id = current_attempt_id;
        run.updated_at_unix = updated_at_unix;
        run.completed_at_unix = completed_at_unix;
        run.validate_facts()?;
        Ok(run)
    }

    pub fn has_same_enqueue_request(&self, other: &Self) -> bool {
        self.idempotency_key == other.idempotency_key
            && self.workflow == other.workflow
            && self.workflow_revision_digest == other.workflow_revision_digest
            && self.trigger == other.trigger
            && self.requested_by_user_id == other.requested_by_user_id
            && self.source == other.source
            && self.desired_runner == other.desired_runner
    }

    #[allow(clippy::too_many_arguments)]
    pub fn claim(
        &mut self,
        runner: &Runner,
        grant: &RunnerGrant,
        attempt_id: impl Into<String>,
        token_hash: impl Into<String>,
        now_unix: u64,
        lease_expires_at_unix: u64,
    ) -> Result<RunAttempt, DomainError> {
        if self.state != RunState::Queued || self.current_attempt_id.is_some() {
            return Err(DomainError::conflict("run is not queued"));
        }
        if self.cancellation_requested {
            return Err(DomainError::conflict("run cancellation is requested"));
        }
        if !runner.supports_dispatch() {
            return Err(DomainError::conflict(
                "runner does not support the V1 dispatch protocol",
            ));
        }
        if !grant.is_active()
            || grant.repository_id != self.workflow.repository_id()
            || grant.runner_id != runner.id
            || !self.desired_runner.matches_name(grant.name.as_str())
        {
            return Err(DomainError::forbidden(
                "runner is not eligible for this run",
            ));
        }
        if lease_expires_at_unix <= now_unix {
            return Err(DomainError::invalid_input(
                "attempt lease expiry must be in the future",
            ));
        }
        self.ensure_time_not_before_update(now_unix)?;
        let attempt_id = required("run attempt id", attempt_id.into())?;
        let token_hash = token_hash.into();
        validate_sha256_hash("attempt token hash", &token_hash)?;
        let number = self
            .last_attempt_number
            .checked_add(1)
            .ok_or_else(|| DomainError::conflict("run attempt number overflow"))?;
        let attempt = RunAttempt {
            id: attempt_id.clone(),
            run_id: self.id.clone(),
            number,
            runner_id: runner.id.clone(),
            token_hash,
            token_expires_at_unix: lease_expires_at_unix,
            state: AttemptState::Leased,
            lease_expires_at_unix,
            last_heartbeat_at_unix: now_unix,
            created_at_unix: now_unix,
            started_at_unix: None,
            completed_at_unix: None,
            exit_code: None,
            log_bytes: 0,
            logs_truncated: false,
        };
        self.state = RunState::Leased;
        self.last_attempt_number = number;
        self.current_attempt_id = Some(attempt_id);
        self.updated_at_unix = now_unix;
        Ok(attempt)
    }

    pub fn request_cancellation(&mut self, now_unix: u64) -> Result<bool, DomainError> {
        if self.state.is_terminal() {
            return Ok(false);
        }
        self.ensure_time_not_before_update(now_unix)?;
        if self.state == RunState::Queued {
            self.state = RunState::Canceled;
            self.cancellation_requested = true;
            self.updated_at_unix = now_unix;
            self.completed_at_unix = Some(now_unix);
            return Ok(true);
        }
        if self.cancellation_requested {
            return Ok(false);
        }
        self.cancellation_requested = true;
        self.updated_at_unix = now_unix;
        Ok(true)
    }

    pub fn pin_container_image(
        &mut self,
        image: PinnedContainerImage,
        now_unix: u64,
    ) -> Result<bool, DomainError> {
        if self.state != RunState::Leased {
            return Err(DomainError::conflict(
                "container image can only be pinned for a leased run",
            ));
        }
        self.ensure_time_not_before_update(now_unix)?;
        if let Some(existing) = &self.pinned_container_image {
            if existing != &image {
                return Err(DomainError::conflict(
                    "run container image is already pinned to a different digest",
                ));
            }
            return Ok(false);
        }
        self.pinned_container_image = Some(image);
        self.updated_at_unix = now_unix;
        Ok(true)
    }

    pub fn validate_workflow_revision(
        &self,
        revision: &WorkflowRevision,
    ) -> Result<(), DomainError> {
        if revision.workflow() != &self.workflow
            || revision.digest() != self.workflow_revision_digest
        {
            return Err(DomainError::invalid_input(
                "run workflow revision does not match the run identity",
            ));
        }
        let triggers = revision.definition().triggers();
        let enabled = match self.trigger {
            RunTrigger::Manual => triggers.manual(),
            RunTrigger::PushMain => triggers.push_main(),
        };
        if !enabled {
            return Err(DomainError::invalid_input(
                "run trigger is not enabled by the workflow revision",
            ));
        }
        let workflow_runner = revision.definition().runner();
        // A concrete manual target is a one-run CLI override; push routing remains repository-owned.
        let runner_is_allowed = match self.trigger {
            RunTrigger::PushMain => &self.desired_runner == workflow_runner,
            RunTrigger::Manual => !matches!(
                (&self.desired_runner, workflow_runner),
                (RunnerSelector::Any, RunnerSelector::Named(_))
            ),
        };
        if !runner_is_allowed {
            return Err(DomainError::invalid_input(
                "run runner selection is not allowed by the workflow revision",
            ));
        }
        Ok(())
    }

    pub fn retry(&mut self, now_unix: u64) -> Result<(), DomainError> {
        if !self.state.is_terminal() {
            return Err(DomainError::conflict("only a terminal run can be retried"));
        }
        self.ensure_time_not_before_update(now_unix)?;
        self.state = RunState::Queued;
        self.cancellation_requested = false;
        self.current_attempt_id = None;
        self.updated_at_unix = now_unix;
        self.completed_at_unix = None;
        Ok(())
    }

    fn ensure_current_attempt(&self, attempt: &RunAttempt) -> Result<(), DomainError> {
        if attempt.run_id != self.id
            || attempt.number != self.last_attempt_number
            || self.current_attempt_id.as_deref() != Some(attempt.id.as_str())
        {
            return Err(DomainError::conflict(
                "attempt is not the current run attempt",
            ));
        }
        Ok(())
    }

    fn ensure_time_not_before_update(&self, now_unix: u64) -> Result<(), DomainError> {
        if now_unix < self.updated_at_unix {
            return Err(DomainError::invalid_input(
                "run transition time cannot move backward",
            ));
        }
        Ok(())
    }

    fn validate_facts(&self) -> Result<(), DomainError> {
        let active = matches!(self.state, RunState::Leased | RunState::Running);
        if active && self.current_attempt_id.is_none() {
            return Err(DomainError::invariant_violation(
                "active run state and current attempt disagree",
            ));
        }
        if (self.state.is_terminal()) != self.completed_at_unix.is_some() {
            return Err(DomainError::invariant_violation(
                "run terminal state and completion time disagree",
            ));
        }
        if self.state == RunState::Queued
            && (self.cancellation_requested
                || self.current_attempt_id.is_some()
                || self.completed_at_unix.is_some())
        {
            return Err(DomainError::invariant_violation(
                "queued run cannot have cancellation intent, an active attempt, or a completion time",
            ));
        }
        if self.last_attempt_number == 0 && self.current_attempt_id.is_some() {
            return Err(DomainError::invariant_violation(
                "run cannot reference attempt zero",
            ));
        }
        if self.updated_at_unix < self.created_at_unix
            || self
                .completed_at_unix
                .is_some_and(|completed| completed != self.updated_at_unix)
        {
            return Err(DomainError::invariant_violation(
                "run timestamps are inconsistent",
            ));
        }
        if self.state == RunState::Canceled && !self.cancellation_requested {
            return Err(DomainError::invariant_violation(
                "canceled run must record cancellation intent",
            ));
        }
        if matches!(self.state, RunState::Running | RunState::Succeeded)
            && self.pinned_container_image.is_none()
        {
            return Err(DomainError::invariant_violation(
                "an executed run must have an immutable container image",
            ));
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RunAttempt {
    pub id: String,
    pub run_id: String,
    pub number: u32,
    pub runner_id: String,
    pub token_hash: String,
    pub token_expires_at_unix: u64,
    pub state: AttemptState,
    pub lease_expires_at_unix: u64,
    pub last_heartbeat_at_unix: u64,
    pub created_at_unix: u64,
    pub started_at_unix: Option<u64>,
    pub completed_at_unix: Option<u64>,
    pub exit_code: Option<i32>,
    pub log_bytes: u64,
    pub logs_truncated: bool,
}

impl RunAttempt {
    #[allow(clippy::too_many_arguments)]
    pub fn restore(
        id: impl Into<String>,
        run_id: impl Into<String>,
        number: u32,
        runner_id: impl Into<String>,
        token_hash: impl Into<String>,
        token_expires_at_unix: u64,
        state: AttemptState,
        lease_expires_at_unix: u64,
        last_heartbeat_at_unix: u64,
        created_at_unix: u64,
        started_at_unix: Option<u64>,
        completed_at_unix: Option<u64>,
        exit_code: Option<i32>,
        log_bytes: u64,
        logs_truncated: bool,
    ) -> Result<Self, DomainError> {
        let token_hash = token_hash.into();
        validate_sha256_hash("attempt token hash", &token_hash)?;
        let attempt = Self {
            id: required("run attempt id", id.into())?,
            run_id: required("run attempt run id", run_id.into())?,
            number,
            runner_id: required("run attempt runner id", runner_id.into())?,
            token_hash,
            token_expires_at_unix,
            state,
            lease_expires_at_unix,
            last_heartbeat_at_unix,
            created_at_unix,
            started_at_unix,
            completed_at_unix,
            exit_code,
            log_bytes,
            logs_truncated,
        };
        attempt.validate_facts()?;
        Ok(attempt)
    }

    pub fn start(
        &mut self,
        run: &mut Run,
        runner_id: &str,
        token_hash: &str,
        now_unix: u64,
    ) -> Result<(), DomainError> {
        self.authenticate(run, runner_id, token_hash, now_unix)?;
        if self.state != AttemptState::Leased || run.state != RunState::Leased {
            return Err(DomainError::conflict("attempt is not leased"));
        }
        if run.pinned_container_image.is_none() {
            return Err(DomainError::conflict(
                "run container image must be pinned before execution starts",
            ));
        }
        if run.cancellation_requested {
            return Err(DomainError::conflict("run cancellation is requested"));
        }
        self.ensure_time_not_before_heartbeat(now_unix)?;
        run.ensure_time_not_before_update(now_unix)?;
        self.state = AttemptState::Running;
        self.started_at_unix = Some(now_unix);
        run.state = RunState::Running;
        run.updated_at_unix = now_unix;
        Ok(())
    }

    pub fn heartbeat(
        &mut self,
        run: &Run,
        runner_id: &str,
        token_hash: &str,
        now_unix: u64,
        lease_expires_at_unix: u64,
    ) -> Result<bool, DomainError> {
        self.authenticate(run, runner_id, token_hash, now_unix)?;
        if self.state.is_terminal() {
            return Err(DomainError::conflict("attempt is terminal"));
        }
        if lease_expires_at_unix <= now_unix {
            return Err(DomainError::invalid_input(
                "attempt lease expiry must be in the future",
            ));
        }
        self.ensure_time_not_before_heartbeat(now_unix)?;
        if lease_expires_at_unix < self.lease_expires_at_unix {
            return Err(DomainError::invalid_input(
                "attempt heartbeat cannot shorten its lease",
            ));
        }
        self.last_heartbeat_at_unix = now_unix;
        self.lease_expires_at_unix = lease_expires_at_unix;
        self.token_expires_at_unix = lease_expires_at_unix;
        Ok(run.cancellation_requested)
    }

    pub fn accept_log_chunk(&mut self, chunk: &RunLogChunk) -> Result<bool, DomainError> {
        if chunk.attempt_id != self.id {
            return Err(DomainError::invalid_input(
                "run log attempt id does not match the attempt",
            ));
        }
        if self.logs_truncated {
            return Ok(false);
        }
        let next_bytes = self
            .log_bytes
            .checked_add(chunk.text.len() as u64)
            .ok_or_else(|| DomainError::conflict("run log byte count overflow"))?;
        if next_bytes > MAX_RUN_LOG_BYTES_PER_ATTEMPT {
            self.logs_truncated = true;
            return Ok(false);
        }
        self.log_bytes = next_bytes;
        Ok(true)
    }

    pub fn authenticate_access(
        &self,
        run: &Run,
        token_hash: &str,
        now_unix: u64,
    ) -> Result<(), DomainError> {
        self.authenticate(run, &self.runner_id, token_hash, now_unix)
    }

    pub fn complete(
        &mut self,
        run: &mut Run,
        runner_id: &str,
        token_hash: &str,
        conclusion: AttemptConclusion,
        now_unix: u64,
    ) -> Result<(), DomainError> {
        self.authenticate(run, runner_id, token_hash, now_unix)?;
        if self.state.is_terminal() || run.state.is_terminal() {
            return Err(DomainError::conflict("attempt is terminal"));
        }
        self.ensure_time_not_before_heartbeat(now_unix)?;
        run.ensure_time_not_before_update(now_unix)?;
        let (attempt_state, run_state, exit_code) = match conclusion {
            AttemptConclusion::Succeeded => {
                if self.state != AttemptState::Running {
                    return Err(DomainError::conflict("only a running attempt can succeed"));
                }
                (AttemptState::Succeeded, RunState::Succeeded, Some(0))
            }
            AttemptConclusion::Failed { exit_code } => {
                if exit_code == 0 {
                    return Err(DomainError::invalid_input(
                        "failed attempt exit code cannot be zero",
                    ));
                }
                (AttemptState::Failed, RunState::Failed, Some(exit_code))
            }
            AttemptConclusion::Canceled => {
                if !run.cancellation_requested {
                    return Err(DomainError::conflict("run cancellation was not requested"));
                }
                (AttemptState::Canceled, RunState::Canceled, None)
            }
        };
        self.state = attempt_state;
        self.exit_code = exit_code;
        self.completed_at_unix = Some(now_unix);
        run.state = run_state;
        run.updated_at_unix = now_unix;
        run.completed_at_unix = Some(now_unix);
        Ok(())
    }

    pub fn expire(&mut self, run: &mut Run, now_unix: u64) -> Result<(), DomainError> {
        run.ensure_current_attempt(self)?;
        if self.state.is_terminal() {
            return Err(DomainError::conflict("attempt is terminal"));
        }
        if now_unix < self.lease_expires_at_unix {
            return Err(DomainError::conflict("attempt lease has not expired"));
        }
        self.finish_lost_or_requeue(run, now_unix)
    }

    pub fn abandon(
        &mut self,
        run: &mut Run,
        runner_id: &str,
        token_hash: &str,
        now_unix: u64,
    ) -> Result<(), DomainError> {
        self.authenticate(run, runner_id, token_hash, now_unix)?;
        if self.state.is_terminal() {
            return Ok(());
        }
        self.finish_lost_or_requeue(run, now_unix)
    }

    fn finish_lost_or_requeue(&mut self, run: &mut Run, now_unix: u64) -> Result<(), DomainError> {
        run.ensure_current_attempt(self)?;
        run.ensure_time_not_before_update(now_unix)?;
        let was_running = self.state == AttemptState::Running;
        self.state = AttemptState::Lost;
        self.completed_at_unix = Some(now_unix);
        run.current_attempt_id = None;
        run.updated_at_unix = now_unix;
        if was_running {
            run.state = RunState::Lost;
            run.completed_at_unix = Some(now_unix);
        } else if run.cancellation_requested {
            run.state = RunState::Canceled;
            run.completed_at_unix = Some(now_unix);
        } else {
            run.state = RunState::Queued;
        }
        Ok(())
    }

    fn authenticate(
        &self,
        run: &Run,
        runner_id: &str,
        token_hash: &str,
        now_unix: u64,
    ) -> Result<(), DomainError> {
        run.ensure_current_attempt(self)?;
        let states_align = matches!(
            (run.state, self.state),
            (RunState::Leased, AttemptState::Leased)
                | (RunState::Running, AttemptState::Running)
                | (RunState::Succeeded, AttemptState::Succeeded)
                | (RunState::Failed, AttemptState::Failed)
                | (RunState::Canceled, AttemptState::Canceled)
        );
        if !states_align {
            return Err(DomainError::invariant_violation(
                "run and attempt states disagree",
            ));
        }
        if self.runner_id != runner_id
            || self.token_hash != token_hash
            || now_unix >= self.token_expires_at_unix
        {
            return Err(DomainError::authentication_failed(
                "attempt credentials are invalid or expired",
            ));
        }
        Ok(())
    }

    fn ensure_time_not_before_heartbeat(&self, now_unix: u64) -> Result<(), DomainError> {
        if now_unix < self.last_heartbeat_at_unix {
            return Err(DomainError::invalid_input(
                "attempt transition time cannot move backward",
            ));
        }
        Ok(())
    }

    fn validate_facts(&self) -> Result<(), DomainError> {
        if self.number == 0 {
            return Err(DomainError::invariant_violation(
                "run attempt number must be positive",
            ));
        }
        if self.log_bytes > MAX_RUN_LOG_BYTES_PER_ATTEMPT {
            return Err(DomainError::invariant_violation(
                "run attempt log byte count exceeds its limit",
            ));
        }
        if (self.state == AttemptState::Running || self.state.is_terminal())
            && self.started_at_unix.is_none()
            && matches!(self.state, AttemptState::Running | AttemptState::Succeeded)
        {
            return Err(DomainError::invariant_violation(
                "running or successful attempt must have a start time",
            ));
        }
        if self.state.is_terminal() != self.completed_at_unix.is_some() {
            return Err(DomainError::invariant_violation(
                "attempt terminal state and completion time disagree",
            ));
        }
        if self.token_expires_at_unix != self.lease_expires_at_unix
            || self.last_heartbeat_at_unix < self.created_at_unix
            || self.last_heartbeat_at_unix >= self.lease_expires_at_unix
            || self.started_at_unix.is_some_and(|started| {
                started < self.created_at_unix || started >= self.lease_expires_at_unix
            })
            || self
                .completed_at_unix
                .is_some_and(|completed| completed < self.last_heartbeat_at_unix)
            || self
                .started_at_unix
                .zip(self.completed_at_unix)
                .is_some_and(|(started, completed)| completed < started)
        {
            return Err(DomainError::invariant_violation(
                "run attempt timestamps are inconsistent",
            ));
        }
        match self.state {
            AttemptState::Succeeded if self.exit_code != Some(0) => {
                return Err(DomainError::invariant_violation(
                    "successful attempt must have exit code zero",
                ));
            }
            AttemptState::Failed if self.exit_code.is_none_or(|code| code == 0) => {
                return Err(DomainError::invariant_violation(
                    "failed attempt must have a nonzero exit code",
                ));
            }
            AttemptState::Leased
            | AttemptState::Running
            | AttemptState::Canceled
            | AttemptState::Lost
                if self.exit_code.is_some() =>
            {
                return Err(DomainError::invariant_violation(
                    "non-failed attempt cannot have an exit code",
                ));
            }
            _ => {}
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests;
