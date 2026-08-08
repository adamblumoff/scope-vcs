use super::{
    run::{MAX_RUN_ATTEMPTS, PinnedContainerImage, Run, RunAttempt, RunAttemptStep},
    runner::{Runner, RunnerGrant, required, validate_sha256_hash},
    state::{RunJobState, RunState},
    workflow::{RunnerSelector, WorkflowJob, WorkflowJobId, WorkflowRevision},
};
use crate::error::DomainError;
use std::collections::BTreeMap;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RunJob {
    pub run_id: String,
    pub key: WorkflowJobId,
    pub desired_runner: RunnerSelector,
    pub pinned_container_image: Option<PinnedContainerImage>,
    pub state: RunJobState,
    pub last_attempt_number: u32,
    pub current_attempt_id: Option<String>,
    pub created_at_unix: u64,
    pub updated_at_unix: u64,
    pub completed_at_unix: Option<u64>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RunRunnerSummary {
    Any,
    Named(String),
    Mixed,
}

pub fn summarize_run_runners(run: &Run, jobs: &[RunJob]) -> Result<RunRunnerSummary, DomainError> {
    let first = jobs.first().ok_or_else(|| {
        DomainError::invariant_violation("run runner summary requires persisted jobs")
    })?;
    if jobs.iter().any(|job| job.run_id != run.id) {
        return Err(DomainError::invariant_violation(
            "run runner summary contains a job from another run",
        ));
    }
    if jobs
        .iter()
        .skip(1)
        .any(|job| job.desired_runner != first.desired_runner)
    {
        return Ok(RunRunnerSummary::Mixed);
    }
    Ok(match &first.desired_runner {
        RunnerSelector::Any => RunRunnerSummary::Any,
        RunnerSelector::Named(name) => RunRunnerSummary::Named(name.clone()),
    })
}

impl RunJob {
    pub fn new(run: &Run, definition: &WorkflowJob) -> Result<Self, DomainError> {
        let desired_runner = run
            .runner_override
            .clone()
            .unwrap_or_else(|| definition.runner().clone());
        desired_runner
            .validate()
            .map_err(|error| DomainError::invalid_input(error.to_string()))?;
        Ok(Self {
            run_id: run.id.clone(),
            key: definition.id().clone(),
            desired_runner,
            pinned_container_image: None,
            state: if definition.needs().is_empty() {
                RunJobState::Queued
            } else {
                RunJobState::Blocked
            },
            last_attempt_number: 0,
            current_attempt_id: None,
            created_at_unix: run.created_at_unix,
            updated_at_unix: run.created_at_unix,
            completed_at_unix: None,
        })
    }

    #[allow(clippy::too_many_arguments)]
    pub fn restore(
        run_id: impl Into<String>,
        key: WorkflowJobId,
        desired_runner: RunnerSelector,
        pinned_container_image: Option<PinnedContainerImage>,
        state: RunJobState,
        last_attempt_number: u32,
        current_attempt_id: Option<String>,
        created_at_unix: u64,
        updated_at_unix: u64,
        completed_at_unix: Option<u64>,
    ) -> Result<Self, DomainError> {
        let job = Self {
            run_id: required("run job run id", run_id.into())?,
            key,
            desired_runner,
            pinned_container_image,
            state,
            last_attempt_number,
            current_attempt_id,
            created_at_unix,
            updated_at_unix,
            completed_at_unix,
        };
        job.validate_facts()?;
        Ok(job)
    }

    #[allow(clippy::too_many_arguments)]
    pub fn claim(
        &mut self,
        run: &Run,
        definition: &WorkflowJob,
        runner: &Runner,
        grant: &RunnerGrant,
        attempt_id: impl Into<String>,
        token_hash: impl Into<String>,
        now_unix: u64,
        lease_expires_at_unix: u64,
    ) -> Result<(RunAttempt, Vec<RunAttemptStep>), DomainError> {
        if self.run_id != run.id || self.key != *definition.id() {
            return Err(DomainError::invariant_violation(
                "run job does not match its run or workflow definition",
            ));
        }
        if self.state != RunJobState::Queued || self.current_attempt_id.is_some() {
            return Err(DomainError::conflict("run job is not queued"));
        }
        if run.cancellation_requested {
            return Err(DomainError::conflict("run cancellation is requested"));
        }
        if !runner.supports_dispatch() {
            return Err(DomainError::conflict(
                "runner does not support the V6 dispatch protocol",
            ));
        }
        if !grant.is_active()
            || grant.repository_id != run.workflow.repository_id()
            || grant.runner_id != runner.id
            || !self.desired_runner.matches_name(grant.name.as_str())
        {
            return Err(DomainError::forbidden(
                "runner is not eligible for this run job",
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
            .ok_or_else(|| DomainError::conflict("run job attempt number overflow"))?;
        if number > MAX_RUN_ATTEMPTS {
            return Err(DomainError::conflict("run job attempt limit reached"));
        }
        let attempt = RunAttempt {
            id: attempt_id.clone(),
            run_id: run.id.clone(),
            job_key: self.key.clone(),
            number,
            runner_id: runner.id.clone(),
            runner_name: grant.name.as_str().to_string(),
            token_hash,
            token_expires_at_unix: lease_expires_at_unix,
            state: super::state::AttemptState::Leased,
            lease_expires_at_unix,
            last_heartbeat_at_unix: now_unix,
            created_at_unix: now_unix,
            started_at_unix: None,
            completed_at_unix: None,
            terminal_reason: None,
            log_bytes: 0,
            logs_truncated: false,
        };
        let steps = definition
            .steps()
            .iter()
            .enumerate()
            .map(|(index, _)| {
                let index = u32::try_from(index)
                    .map_err(|_| DomainError::conflict("workflow step index overflow"))?;
                RunAttemptStep::pending(attempt_id.clone(), index)
            })
            .collect::<Result<Vec<_>, _>>()?;
        self.state = RunJobState::Leased;
        self.last_attempt_number = number;
        self.current_attempt_id = Some(attempt_id);
        self.updated_at_unix = now_unix;
        Ok((attempt, steps))
    }

    pub fn pin_container_image(
        &mut self,
        image: PinnedContainerImage,
        now_unix: u64,
    ) -> Result<bool, DomainError> {
        if self.state != RunJobState::Leased {
            return Err(DomainError::conflict(
                "container image can only be pinned for a leased run job",
            ));
        }
        self.ensure_time_not_before_update(now_unix)?;
        if let Some(existing) = &self.pinned_container_image {
            if existing != &image {
                return Err(DomainError::conflict(
                    "run job container image is already pinned to a different digest",
                ));
            }
            return Ok(false);
        }
        self.pinned_container_image = Some(image);
        self.updated_at_unix = now_unix;
        Ok(true)
    }

    pub(crate) fn ensure_current_attempt(&self, attempt: &RunAttempt) -> Result<(), DomainError> {
        self.ensure_latest_attempt(attempt)?;
        if self.current_attempt_id.as_deref() != Some(attempt.id.as_str()) {
            return Err(DomainError::conflict(
                "attempt is not the current run job attempt",
            ));
        }
        Ok(())
    }

    pub(crate) fn ensure_latest_attempt(&self, attempt: &RunAttempt) -> Result<(), DomainError> {
        if attempt.run_id != self.run_id
            || attempt.job_key != self.key
            || attempt.number != self.last_attempt_number
        {
            return Err(DomainError::conflict(
                "attempt is not the latest run job attempt",
            ));
        }
        Ok(())
    }

    pub(crate) fn ensure_time_not_before_update(&self, now_unix: u64) -> Result<(), DomainError> {
        if now_unix < self.updated_at_unix {
            return Err(DomainError::invalid_input(
                "run job transition time cannot move backward",
            ));
        }
        Ok(())
    }

    fn validate_facts(&self) -> Result<(), DomainError> {
        let active = matches!(self.state, RunJobState::Leased | RunJobState::Running);
        if active != self.current_attempt_id.is_some() {
            return Err(DomainError::invariant_violation(
                "run job active state and current attempt disagree",
            ));
        }
        if self.state.is_terminal() != self.completed_at_unix.is_some() {
            return Err(DomainError::invariant_violation(
                "run job terminal state and completion time disagree",
            ));
        }
        if self.last_attempt_number > MAX_RUN_ATTEMPTS {
            return Err(DomainError::invariant_violation(
                "run job attempt history exceeds its bounded limit",
            ));
        }
        if matches!(self.state, RunJobState::Running | RunJobState::Succeeded)
            && self.pinned_container_image.is_none()
        {
            return Err(DomainError::invariant_violation(
                "an executed run job must have an immutable container image",
            ));
        }
        if self.updated_at_unix < self.created_at_unix
            || self
                .completed_at_unix
                .is_some_and(|completed| completed != self.updated_at_unix)
        {
            return Err(DomainError::invariant_violation(
                "run job timestamps are inconsistent",
            ));
        }
        Ok(())
    }
}

pub fn create_run_jobs(run: &Run, revision: &WorkflowRevision) -> Result<Vec<RunJob>, DomainError> {
    run.validate_workflow_revision(revision)?;
    revision
        .definition()
        .jobs()
        .iter()
        .map(|definition| RunJob::new(run, definition))
        .collect()
}

pub fn reconcile_run(
    run: &mut Run,
    jobs: &mut [RunJob],
    revision: &WorkflowRevision,
    now_unix: u64,
) -> Result<(), DomainError> {
    run.validate_workflow_revision(revision)?;
    let job_indexes = jobs
        .iter()
        .enumerate()
        .map(|(index, job)| (job.key.clone(), index))
        .collect::<BTreeMap<_, _>>();
    if job_indexes.len() != revision.definition().jobs().len()
        || revision
            .definition()
            .jobs()
            .iter()
            .any(|definition| !job_indexes.contains_key(definition.id()))
    {
        return Err(DomainError::invariant_violation(
            "persisted run jobs do not match their workflow",
        ));
    }
    // The compiled workflow's serial order is deterministic and topological. Processing it once
    // therefore reaches a fixed point even when a failure skips several dependency levels.
    for definition in revision.definition().serial_jobs() {
        let index = *job_indexes.get(definition.id()).ok_or_else(|| {
            DomainError::invariant_violation("persisted run job is missing from its workflow")
        })?;
        if jobs[index].state != RunJobState::Blocked {
            continue;
        }
        let dependency_facts = definition
            .needs()
            .iter()
            .map(|dependency| {
                job_indexes.get(dependency).map(|dependency_index| {
                    (
                        jobs[*dependency_index].state,
                        jobs[*dependency_index].updated_at_unix,
                    )
                })
            })
            .collect::<Option<Vec<_>>>()
            .ok_or_else(|| DomainError::invariant_violation("run job dependency is missing"))?;
        let next = if dependency_facts
            .iter()
            .any(|(state, _)| state.is_terminal() && *state != RunJobState::Succeeded)
        {
            Some(RunJobState::Skipped)
        } else if dependency_facts
            .iter()
            .all(|(state, _)| *state == RunJobState::Succeeded)
        {
            Some(RunJobState::Queued)
        } else {
            None
        };
        if let Some(next) = next {
            let job = &mut jobs[index];
            let transition_time = dependency_facts
                .iter()
                .fold(now_unix.max(job.updated_at_unix), |latest, (_, updated)| {
                    latest.max(*updated)
                });
            job.state = next;
            job.updated_at_unix = transition_time;
            if next.is_terminal() {
                job.completed_at_unix = Some(transition_time);
            }
        }
    }
    derive_run_state(run, jobs, now_unix)
}

pub fn request_run_cancellation(
    run: &mut Run,
    jobs: &mut [RunJob],
    now_unix: u64,
) -> Result<bool, DomainError> {
    let request_time = now_unix.max(run.updated_at_unix);
    if !run.request_cancellation(request_time)? {
        return Ok(false);
    }
    for job in jobs
        .iter_mut()
        .filter(|job| matches!(job.state, RunJobState::Blocked | RunJobState::Queued))
    {
        let transition_time = now_unix.max(job.updated_at_unix);
        job.state = RunJobState::Canceled;
        job.updated_at_unix = transition_time;
        job.completed_at_unix = Some(transition_time);
    }
    derive_run_state(run, jobs, now_unix)?;
    Ok(true)
}

pub fn retry_run(
    run: &mut Run,
    jobs: &mut [RunJob],
    revision: &WorkflowRevision,
    now_unix: u64,
) -> Result<(), DomainError> {
    run.validate_workflow_revision(revision)?;
    ensure_retry_time(run, jobs, now_unix)?;
    let expected = revision
        .definition()
        .jobs()
        .iter()
        .map(|job| job.id())
        .collect::<std::collections::BTreeSet<_>>();
    let actual = jobs
        .iter()
        .map(|job| &job.key)
        .collect::<std::collections::BTreeSet<_>>();
    if expected != actual || jobs.iter().any(|job| job.run_id != run.id) {
        return Err(DomainError::invariant_violation(
            "persisted run jobs do not match their workflow",
        ));
    }
    if !can_retry_run(run, jobs) {
        return Err(DomainError::conflict("run cannot be retried"));
    }
    for job in jobs.iter_mut() {
        let definition = revision.definition().job(&job.key).ok_or_else(|| {
            DomainError::invariant_violation("persisted run job is missing from its workflow")
        })?;
        job.state = if definition.needs().is_empty() {
            RunJobState::Queued
        } else {
            RunJobState::Blocked
        };
        job.current_attempt_id = None;
        job.updated_at_unix = now_unix;
        job.completed_at_unix = None;
    }
    run.state = RunState::Queued;
    run.cancellation_requested = false;
    run.updated_at_unix = now_unix;
    run.completed_at_unix = None;
    Ok(())
}

pub fn can_retry_run(run: &Run, jobs: &[RunJob]) -> bool {
    run.state.is_terminal()
        && !jobs.is_empty()
        && jobs
            .iter()
            .all(|job| job.run_id == run.id && job.last_attempt_number < MAX_RUN_ATTEMPTS)
}

fn derive_run_state(run: &mut Run, jobs: &mut [RunJob], now_unix: u64) -> Result<(), DomainError> {
    if jobs.is_empty() || jobs.iter().any(|job| job.run_id != run.id) {
        return Err(DomainError::invariant_violation(
            "run jobs do not form one non-empty run",
        ));
    }
    if jobs.iter().any(|job| job.state == RunJobState::Canceled) {
        run.cancellation_requested = true;
    }
    if run.cancellation_requested {
        let cancellation_time = jobs
            .iter()
            .filter(|job| job.state == RunJobState::Canceled)
            .fold(now_unix, |latest, job| latest.max(job.updated_at_unix));
        for job in jobs
            .iter_mut()
            .filter(|job| matches!(job.state, RunJobState::Blocked | RunJobState::Queued))
        {
            let transition_time = cancellation_time.max(job.updated_at_unix);
            job.state = RunJobState::Canceled;
            job.updated_at_unix = transition_time;
            job.completed_at_unix = Some(transition_time);
        }
    }
    let cancellation_took_effect = jobs.iter().any(|job| job.state == RunJobState::Canceled);
    let all_terminal = jobs.iter().all(|job| job.state.is_terminal());
    let next = if all_terminal {
        if cancellation_took_effect {
            RunState::Canceled
        } else if jobs.iter().any(|job| job.state == RunJobState::Failed) {
            RunState::Failed
        } else if jobs.iter().any(|job| job.state == RunJobState::Lost) {
            RunState::Lost
        } else {
            RunState::Succeeded
        }
    } else if run.state == RunState::Running
        || jobs.iter().any(|job| job.state == RunJobState::Running)
    {
        RunState::Running
    } else if jobs.iter().any(|job| job.state == RunJobState::Leased) {
        RunState::Leased
    } else {
        RunState::Queued
    };
    let aggregate_time = jobs
        .iter()
        .fold(now_unix.max(run.updated_at_unix), |latest, job| {
            latest.max(job.updated_at_unix)
        });
    run.state = next;
    if all_terminal && !cancellation_took_effect {
        run.cancellation_requested = false;
    }
    run.updated_at_unix = aggregate_time;
    run.completed_at_unix = next.is_terminal().then_some(aggregate_time);
    Ok(())
}

fn ensure_retry_time(run: &Run, jobs: &[RunJob], now_unix: u64) -> Result<(), DomainError> {
    if now_unix < run.updated_at_unix || jobs.iter().any(|job| now_unix < job.updated_at_unix) {
        return Err(DomainError::invalid_input(
            "run aggregate transition time cannot move backward",
        ));
    }
    Ok(())
}
