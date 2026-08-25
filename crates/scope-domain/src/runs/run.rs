use super::{
    source::{RunSource, RunTrigger},
    validation::{required, validate_sha256_hash},
    workflow::{identity::WorkflowIdentity, revision::WorkflowRevision},
};
use crate::error::DomainError;
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum RunState {
    Queued,
    Dispatching,
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

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Run {
    pub id: String,
    pub idempotency_key: String,
    pub workflow: WorkflowIdentity,
    pub workflow_revision_digest: String,
    pub trigger: RunTrigger,
    pub requested_by_user_id: Option<String>,
    pub source: RunSource,
    pub state: RunState,
    pub cancellation_requested: bool,
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
        now_unix: u64,
    ) -> Result<Self, DomainError> {
        let id = required("run id", id.into())?;
        let idempotency_key = required("run idempotency key", idempotency_key.into())?;
        let workflow_revision_digest = workflow_revision_digest.into();
        validate_sha256_hash("workflow revision digest", &workflow_revision_digest)?;
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
            state: RunState::Queued,
            cancellation_requested: false,
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
        state: RunState,
        cancellation_requested: bool,
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
            created_at_unix,
        )?;
        run.state = state;
        run.cancellation_requested = cancellation_requested;
        run.updated_at_unix = updated_at_unix;
        run.completed_at_unix = completed_at_unix;
        run.validate_facts()?;
        Ok(run)
    }

    pub fn has_same_enqueue_request_identity(&self, other: &Self) -> bool {
        self.idempotency_key == other.idempotency_key
            && self.workflow == other.workflow
            && self.workflow_revision_digest == other.workflow_revision_digest
            && self.trigger == other.trigger
            && self.requested_by_user_id == other.requested_by_user_id
            && self.source == other.source
    }

    pub fn belongs_to_repository(&self, repository_id: &str) -> bool {
        self.workflow.repository_id() == repository_id
    }

    pub fn can_request_cancellation(&self) -> bool {
        !self.state.is_terminal() && !self.cancellation_requested
    }

    pub fn request_cancellation(&mut self, now_unix: u64) -> Result<bool, DomainError> {
        if !self.can_request_cancellation() {
            return Ok(false);
        }
        self.ensure_time_not_before_update(now_unix)?;
        self.cancellation_requested = true;
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
        Ok(())
    }

    pub(crate) fn ensure_time_not_before_update(&self, now_unix: u64) -> Result<(), DomainError> {
        if now_unix < self.updated_at_unix {
            return Err(DomainError::invalid_input(
                "run transition time cannot move backward",
            ));
        }
        Ok(())
    }

    fn validate_facts(&self) -> Result<(), DomainError> {
        if (self.state.is_terminal()) != self.completed_at_unix.is_some() {
            return Err(DomainError::invariant_violation(
                "run terminal state and completion time disagree",
            ));
        }
        if self.state == RunState::Queued && self.completed_at_unix.is_some() {
            return Err(DomainError::invariant_violation(
                "queued run cannot have a completion time",
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
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runs::workflow::identity::WorkflowPath;
    use crate::{content::SourceBlob, content_ref::ContentRef};

    #[test]
    fn run_repository_association_comes_from_its_workflow_identity() {
        let run = Run::new(
            "run-1",
            "manual:run-1",
            WorkflowIdentity::new(
                "owner/repo",
                WorkflowPath::parse("/.scope/runs/checks.yml").unwrap(),
            )
            .unwrap(),
            "a".repeat(64),
            RunTrigger::Manual,
            Some("user-1".to_string()),
            RunSource::ephemeral_git_bundle(SourceBlob {
                content_ref: ContentRef::git_bundle_sha256("b".repeat(64)),
                sha256: "b".repeat(64),
                git_oid: "c".repeat(40),
                git_file_mode: "100644".to_string(),
                size_bytes: 1,
            })
            .unwrap(),
            1,
        )
        .unwrap();

        assert!(run.belongs_to_repository("owner/repo"));
        assert!(!run.belongs_to_repository("owner/other"));
    }
}
