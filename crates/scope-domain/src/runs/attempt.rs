use super::{
    job::{RunJob, RunJobState},
    log::{MAX_RUN_LOG_BYTES_PER_ATTEMPT, RunLogChunk},
    run::Run,
    step::{
        AttemptConclusion, AttemptTerminalReason, RunAttemptStep, StepState, interrupt_steps,
        skip_pending_steps,
    },
    validation::{required, validate_sha256_hash},
    workflow::definition::WorkflowJobId,
};
use crate::error::DomainError;
use serde::{Deserialize, Serialize};

pub const MAX_RUN_ATTEMPTS: u32 = 100;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ExecutionProvider {
    Northflank,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum AttemptState {
    Dispatching,
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

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RunAttempt {
    pub id: String,
    pub run_id: String,
    pub job_key: WorkflowJobId,
    pub number: u32,
    pub execution_provider: ExecutionProvider,
    pub external_run_id: Option<String>,
    pub runtime_version: String,
    pub token_hash: String,
    pub token_expires_at_unix: u64,
    pub state: AttemptState,
    pub lease_expires_at_unix: u64,
    pub last_heartbeat_at_unix: u64,
    pub created_at_unix: u64,
    pub started_at_unix: Option<u64>,
    pub completed_at_unix: Option<u64>,
    pub terminal_reason: Option<AttemptTerminalReason>,
    pub log_bytes: u64,
    pub first_truncated_step_index: Option<u32>,
}

impl RunAttempt {
    #[allow(clippy::too_many_arguments)]
    pub fn restore(
        id: impl Into<String>,
        run_id: impl Into<String>,
        job_key: WorkflowJobId,
        number: u32,
        execution_provider: ExecutionProvider,
        external_run_id: Option<String>,
        runtime_version: impl Into<String>,
        token_hash: impl Into<String>,
        token_expires_at_unix: u64,
        state: AttemptState,
        lease_expires_at_unix: u64,
        last_heartbeat_at_unix: u64,
        created_at_unix: u64,
        started_at_unix: Option<u64>,
        completed_at_unix: Option<u64>,
        terminal_reason: Option<AttemptTerminalReason>,
        log_bytes: u64,
        first_truncated_step_index: Option<u32>,
    ) -> Result<Self, DomainError> {
        let token_hash = token_hash.into();
        validate_sha256_hash("attempt token hash", &token_hash)?;
        let attempt = Self {
            id: required("run attempt id", id.into())?,
            run_id: required("run attempt run id", run_id.into())?,
            job_key,
            number,
            execution_provider,
            external_run_id: external_run_id
                .map(|value| required("external run id", value))
                .transpose()?,
            runtime_version: required("runtime version", runtime_version.into())?,
            token_hash,
            token_expires_at_unix,
            state,
            lease_expires_at_unix,
            last_heartbeat_at_unix,
            created_at_unix,
            started_at_unix,
            completed_at_unix,
            terminal_reason,
            log_bytes,
            first_truncated_step_index,
        };
        attempt.validate_facts()?;
        Ok(attempt)
    }

    pub fn start(
        &mut self,
        run: &Run,
        job: &mut RunJob,
        token_hash: &str,
        now_unix: u64,
    ) -> Result<(), DomainError> {
        self.authenticate(job, token_hash, now_unix)?;
        if self.state != AttemptState::Dispatching || job.state != RunJobState::Dispatching {
            return Err(DomainError::conflict("attempt is not dispatching"));
        }
        if run.cancellation_requested {
            return Err(DomainError::conflict("run cancellation is requested"));
        }
        self.ensure_time_not_before_heartbeat(now_unix)?;
        job.ensure_time_not_before_update(now_unix)?;
        self.state = AttemptState::Running;
        self.started_at_unix = Some(now_unix);
        job.state = RunJobState::Running;
        job.updated_at_unix = now_unix;
        Ok(())
    }

    pub fn claim_runtime(
        &mut self,
        job: &RunJob,
        bootstrap_token_hash: &str,
        attempt_token_hash: impl Into<String>,
        now_unix: u64,
        lease_expires_at_unix: u64,
    ) -> Result<(), DomainError> {
        self.authenticate(job, bootstrap_token_hash, now_unix)?;
        if self.state != AttemptState::Dispatching {
            return Err(DomainError::conflict("attempt is not dispatching"));
        }
        if lease_expires_at_unix <= now_unix {
            return Err(DomainError::invalid_input(
                "attempt lease expiry must be in the future",
            ));
        }
        let attempt_token_hash = attempt_token_hash.into();
        validate_sha256_hash("attempt token hash", &attempt_token_hash)?;
        if attempt_token_hash == self.token_hash {
            return Err(DomainError::invalid_input(
                "attempt token must differ from the bootstrap token",
            ));
        }
        self.token_hash = attempt_token_hash;
        self.last_heartbeat_at_unix = now_unix;
        self.token_expires_at_unix = lease_expires_at_unix;
        self.lease_expires_at_unix = lease_expires_at_unix;
        Ok(())
    }

    pub fn heartbeat(
        &mut self,
        run: &Run,
        job: &RunJob,
        token_hash: &str,
        now_unix: u64,
        lease_expires_at_unix: u64,
    ) -> Result<bool, DomainError> {
        self.authenticate(job, token_hash, now_unix)?;
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

    pub fn accept_log_chunk(
        &mut self,
        steps: &[RunAttemptStep],
        chunk: &RunLogChunk,
    ) -> Result<bool, DomainError> {
        if chunk.attempt_id != self.id {
            return Err(DomainError::invalid_input(
                "run log attempt id does not match the attempt",
            ));
        }
        if self.state.is_terminal() {
            return Err(DomainError::conflict(
                "terminal attempts cannot append logs",
            ));
        }
        self.validate_steps(steps)?;
        if steps
            .get(chunk.step_index as usize)
            .is_none_or(|step| step.state != StepState::Running)
        {
            return Err(DomainError::conflict(
                "logs can only be appended to the running step",
            ));
        }
        if self.first_truncated_step_index.is_some() {
            return Ok(false);
        }
        let next_bytes = self
            .log_bytes
            .checked_add(chunk.text.len() as u64)
            .ok_or_else(|| DomainError::conflict("run log byte count overflow"))?;
        if next_bytes > MAX_RUN_LOG_BYTES_PER_ATTEMPT {
            self.first_truncated_step_index = Some(chunk.step_index);
            return Ok(false);
        }
        self.log_bytes = next_bytes;
        Ok(true)
    }

    pub fn mark_step_logs_truncated(
        &mut self,
        steps: &[RunAttemptStep],
        step_index: u32,
    ) -> Result<(), DomainError> {
        self.validate_steps(steps)?;
        let step = steps
            .get(step_index as usize)
            .ok_or_else(|| DomainError::invalid_input("workflow step does not exist"))?;
        if step.state != StepState::Running {
            return Err(DomainError::conflict(
                "only the running step can report truncated logs",
            ));
        }
        match self.first_truncated_step_index {
            Some(existing) if existing <= step_index => Ok(()),
            Some(_) => Err(DomainError::conflict(
                "run attempt log truncation cannot move to an earlier step",
            )),
            None => {
                self.first_truncated_step_index = Some(step_index);
                Ok(())
            }
        }
    }

    pub fn authenticate_access(
        &self,
        job: &RunJob,
        token_hash: &str,
        now_unix: u64,
    ) -> Result<(), DomainError> {
        self.authenticate(job, token_hash, now_unix)
    }

    pub fn authorize_cache_access(&self, job: &RunJob, now_unix: u64) -> Result<(), DomainError> {
        if self.state.is_terminal() {
            return Err(DomainError::authentication_failed(
                "terminal attempts cannot use cache grants",
            ));
        }
        self.ensure_job_alignment(job)?;
        job.ensure_current_attempt(self)?;
        self.ensure_lease_active(now_unix)
    }

    pub fn authenticate_cache_observation_report(
        &self,
        job: &RunJob,
        token_hash: &str,
        now_unix: u64,
    ) -> Result<(), DomainError> {
        if self.state.is_terminal() {
            if self.token_hash != token_hash {
                return Err(DomainError::authentication_failed(
                    "attempt credentials are invalid",
                ));
            }
            Ok(())
        } else {
            self.authenticate_access(job, token_hash, now_unix)
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub fn complete(
        &mut self,
        run: &Run,
        job: &mut RunJob,
        steps: &mut [RunAttemptStep],
        token_hash: &str,
        conclusion: AttemptConclusion,
        logs_truncated: bool,
        now_unix: u64,
    ) -> Result<(), DomainError> {
        if self.state.is_terminal() || job.state.is_terminal() {
            self.authenticate_identity(job, token_hash)?;
            return if self.matches_conclusion(&conclusion) {
                Ok(())
            } else {
                Err(DomainError::conflict(
                    "attempt already completed with a different conclusion",
                ))
            };
        }
        self.authenticate(job, token_hash, now_unix)?;
        self.validate_steps(steps)?;
        if logs_truncated && self.first_truncated_step_index.is_none() {
            let step_index = steps
                .iter()
                .position(|step| step.state == StepState::Running)
                .ok_or_else(|| {
                    DomainError::conflict(
                        "attempt log truncation requires an active or previously truncated step",
                    )
                })? as u32;
            self.mark_step_logs_truncated(steps, step_index)?;
        }
        self.ensure_time_not_before_heartbeat(now_unix)?;
        job.ensure_time_not_before_update(now_unix)?;
        let (attempt_state, job_state, terminal_reason) = match conclusion {
            AttemptConclusion::Succeeded => {
                if self.state != AttemptState::Running
                    || steps.iter().any(|step| step.state != StepState::Succeeded)
                {
                    return Err(DomainError::conflict(
                        "runtime can only succeed after every workflow step succeeds",
                    ));
                }
                if run.cancellation_requested {
                    (
                        AttemptState::Canceled,
                        RunJobState::Canceled,
                        Some(AttemptTerminalReason::Canceled { step_index: None }),
                    )
                } else {
                    (AttemptState::Succeeded, RunJobState::Succeeded, None)
                }
            }
            AttemptConclusion::SetupFailed { exit_code, message } => {
                if self.state != AttemptState::Dispatching {
                    return Err(DomainError::conflict(
                        "runtime setup can only fail before execution starts",
                    ));
                }
                if exit_code == 0 {
                    return Err(DomainError::invalid_input(
                        "runtime setup failure exit code cannot be zero",
                    ));
                }
                if !super::step::valid_setup_failure_message(&message) {
                    return Err(DomainError::invalid_input(
                        "runtime setup failure message is required and must not exceed 2048 bytes",
                    ));
                }
                skip_pending_steps(steps, now_unix);
                (
                    AttemptState::Failed,
                    RunJobState::Failed,
                    Some(AttemptTerminalReason::RuntimeSetupFailed { exit_code, message }),
                )
            }
            AttemptConclusion::TimedOut => {
                let step_index = interrupt_steps(steps, StepState::Canceled, now_unix);
                (
                    AttemptState::Failed,
                    RunJobState::Failed,
                    Some(AttemptTerminalReason::TimedOut { step_index }),
                )
            }
            AttemptConclusion::Canceled => {
                if !run.cancellation_requested {
                    return Err(DomainError::conflict("run cancellation was not requested"));
                }
                let step_index = interrupt_steps(steps, StepState::Canceled, now_unix);
                (
                    AttemptState::Canceled,
                    RunJobState::Canceled,
                    Some(AttemptTerminalReason::Canceled { step_index }),
                )
            }
        };
        self.state = attempt_state;
        self.terminal_reason = terminal_reason;
        self.completed_at_unix = Some(now_unix);
        job.state = job_state;
        job.current_attempt_id = None;
        job.updated_at_unix = now_unix;
        job.completed_at_unix = Some(now_unix);
        Ok(())
    }

    fn matches_conclusion(&self, conclusion: &AttemptConclusion) -> bool {
        match (conclusion, &self.terminal_reason) {
            (AttemptConclusion::Succeeded, None) => self.state == AttemptState::Succeeded,
            (AttemptConclusion::Succeeded, Some(AttemptTerminalReason::Canceled { .. })) => {
                self.state == AttemptState::Canceled
            }
            (
                AttemptConclusion::SetupFailed { exit_code, message },
                Some(AttemptTerminalReason::RuntimeSetupFailed {
                    exit_code: existing,
                    message: existing_message,
                }),
            ) => {
                self.state == AttemptState::Failed
                    && exit_code == existing
                    && message == existing_message
            }
            (AttemptConclusion::TimedOut, Some(AttemptTerminalReason::TimedOut { .. })) => {
                self.state == AttemptState::Failed
            }
            (AttemptConclusion::Canceled, Some(AttemptTerminalReason::Canceled { .. })) => {
                self.state == AttemptState::Canceled
            }
            _ => false,
        }
    }

    pub fn expire(
        &mut self,
        run: &Run,
        job: &mut RunJob,
        steps: &mut [RunAttemptStep],
        now_unix: u64,
    ) -> Result<(), DomainError> {
        job.ensure_current_attempt(self)?;
        if self.state.is_terminal() {
            return Err(DomainError::conflict("attempt is terminal"));
        }
        if now_unix < self.lease_expires_at_unix {
            return Err(DomainError::conflict("attempt lease has not expired"));
        }
        self.finish_lost_or_requeue(run, job, steps, now_unix)
    }

    pub fn abandon(
        &mut self,
        run: &Run,
        job: &mut RunJob,
        steps: &mut [RunAttemptStep],
        token_hash: &str,
        now_unix: u64,
    ) -> Result<(), DomainError> {
        self.authenticate(job, token_hash, now_unix)?;
        if self.state.is_terminal() {
            return Ok(());
        }
        self.finish_lost_or_requeue(run, job, steps, now_unix)
    }

    pub fn confirm_provider_cancellation(
        &mut self,
        run: &Run,
        job: &mut RunJob,
        steps: &mut [RunAttemptStep],
        now_unix: u64,
    ) -> Result<(), DomainError> {
        if self.state == AttemptState::Canceled
            && job.state == RunJobState::Canceled
            && job.current_attempt_id.is_none()
        {
            return Ok(());
        }
        job.ensure_current_attempt(self)?;
        if self.state.is_terminal() {
            return Err(DomainError::conflict("attempt is terminal"));
        }
        if !run.cancellation_requested {
            return Err(DomainError::conflict("run cancellation was not requested"));
        }
        self.validate_steps(steps)?;
        job.ensure_time_not_before_update(now_unix)?;
        let step_index = interrupt_steps(steps, StepState::Canceled, now_unix);
        self.state = AttemptState::Canceled;
        self.terminal_reason = Some(AttemptTerminalReason::Canceled { step_index });
        self.completed_at_unix = Some(now_unix);
        job.state = RunJobState::Canceled;
        job.current_attempt_id = None;
        job.updated_at_unix = now_unix;
        job.completed_at_unix = Some(now_unix);
        Ok(())
    }

    fn finish_lost_or_requeue(
        &mut self,
        run: &Run,
        job: &mut RunJob,
        steps: &mut [RunAttemptStep],
        now_unix: u64,
    ) -> Result<(), DomainError> {
        job.ensure_current_attempt(self)?;
        self.validate_steps(steps)?;
        job.ensure_time_not_before_update(now_unix)?;
        let was_running = self.state == AttemptState::Running;
        let step_index = interrupt_steps(steps, StepState::Lost, now_unix);
        self.state = AttemptState::Lost;
        self.terminal_reason = Some(AttemptTerminalReason::ExecutionLost { step_index });
        self.completed_at_unix = Some(now_unix);
        job.current_attempt_id = None;
        job.updated_at_unix = now_unix;
        if was_running {
            job.state = RunJobState::Lost;
            job.completed_at_unix = Some(now_unix);
        } else if run.cancellation_requested {
            job.state = RunJobState::Canceled;
            job.completed_at_unix = Some(now_unix);
        } else {
            job.state = RunJobState::Queued;
        }
        Ok(())
    }

    pub(crate) fn authenticate(
        &self,
        job: &RunJob,
        token_hash: &str,
        now_unix: u64,
    ) -> Result<(), DomainError> {
        self.authenticate_identity(job, token_hash)?;
        job.ensure_current_attempt(self)?;
        self.ensure_lease_active(now_unix)
    }

    pub(crate) fn authenticate_identity(
        &self,
        job: &RunJob,
        token_hash: &str,
    ) -> Result<(), DomainError> {
        self.ensure_job_alignment(job)?;
        if self.token_hash != token_hash {
            return Err(DomainError::authentication_failed(
                "attempt credentials are invalid",
            ));
        }
        Ok(())
    }

    fn ensure_job_alignment(&self, job: &RunJob) -> Result<(), DomainError> {
        job.ensure_latest_attempt(self)?;
        let states_align = matches!(
            (job.state, self.state),
            (RunJobState::Dispatching, AttemptState::Dispatching)
                | (RunJobState::Running, AttemptState::Running)
                | (RunJobState::Succeeded, AttemptState::Succeeded)
                | (RunJobState::Failed, AttemptState::Failed)
                | (RunJobState::Canceled, AttemptState::Canceled)
                | (RunJobState::Lost, AttemptState::Lost)
        );
        if !states_align {
            return Err(DomainError::invariant_violation(
                "run and attempt states disagree",
            ));
        }
        Ok(())
    }

    fn ensure_lease_active(&self, now_unix: u64) -> Result<(), DomainError> {
        if now_unix >= self.token_expires_at_unix {
            return Err(DomainError::authentication_failed(
                "attempt credentials are invalid or expired",
            ));
        }
        Ok(())
    }

    pub(crate) fn ensure_time_not_before_heartbeat(
        &self,
        now_unix: u64,
    ) -> Result<(), DomainError> {
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
            AttemptState::Succeeded if self.terminal_reason.is_some() => {
                return Err(DomainError::invariant_violation(
                    "successful attempt cannot have a terminal failure reason",
                ));
            }
            AttemptState::Failed | AttemptState::Canceled | AttemptState::Lost
                if self.terminal_reason.is_none() =>
            {
                return Err(DomainError::invariant_violation(
                    "unsuccessful terminal attempt must have a terminal reason",
                ));
            }
            AttemptState::Dispatching | AttemptState::Running if self.terminal_reason.is_some() => {
                return Err(DomainError::invariant_violation(
                    "active attempt cannot have a terminal reason",
                ));
            }
            _ => {}
        }
        Ok(())
    }
}

#[cfg(test)]
mod job_tests;
