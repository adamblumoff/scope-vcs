use super::workflow::{WorkflowError, WorkflowPath};
use crate::error::DomainError;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PushWorkflowFile {
    pub path: String,
    pub bytes: Vec<u8>,
}

impl PushWorkflowFile {
    pub fn new(path: impl Into<String>, bytes: Vec<u8>) -> Result<Self, WorkflowError> {
        let path = WorkflowPath::parse(path.into())?;
        Ok(Self {
            path: path.as_str().to_string(),
            bytes,
        })
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PushTriggerInput {
    pub head_oid: String,
    pub workflows: Vec<PushWorkflowFile>,
    pub configuration_error: Option<String>,
}

impl PushTriggerInput {
    pub fn new(
        head_oid: impl Into<String>,
        workflows: Vec<PushWorkflowFile>,
        configuration_error: Option<String>,
    ) -> Result<Self, DomainError> {
        let head_oid = head_oid.into();
        if head_oid.len() != 40 || !head_oid.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            return Err(DomainError::invalid_input(
                "push trigger head must be a SHA-1 hex digest",
            ));
        }
        Ok(Self {
            head_oid,
            workflows,
            configuration_error,
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum PushTriggerEvaluationState {
    Pending,
    Succeeded,
    ConfigurationError,
    Failed,
}

impl PushTriggerEvaluationState {
    pub fn is_terminal(self) -> bool {
        !matches!(self, Self::Pending)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PushTriggerCheck {
    pub workflow_path: String,
    pub workflow_name: String,
    pub run_id: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PushTriggerEvaluation {
    pub repository_id: String,
    pub change_version: u64,
    pub head_oid: String,
    pub state: PushTriggerEvaluationState,
    pub message: Option<String>,
    pub checks: Vec<PushTriggerCheck>,
    pub created_at_unix: u64,
    pub completed_at_unix: Option<u64>,
}

impl PushTriggerEvaluation {
    pub fn pending(
        repository_id: impl Into<String>,
        change_version: u64,
        head_oid: impl Into<String>,
        now_unix: u64,
    ) -> Result<Self, DomainError> {
        let repository_id = repository_id.into();
        let head_oid = head_oid.into();
        if repository_id.trim().is_empty() || change_version == 0 {
            return Err(DomainError::invalid_input(
                "push trigger evaluation identity is invalid",
            ));
        }
        if head_oid.len() != 40 || !head_oid.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            return Err(DomainError::invalid_input(
                "push trigger evaluation head must be a SHA-1 hex digest",
            ));
        }
        Ok(Self {
            repository_id,
            change_version,
            head_oid,
            state: PushTriggerEvaluationState::Pending,
            message: None,
            checks: Vec::new(),
            created_at_unix: now_unix,
            completed_at_unix: None,
        })
    }

    pub fn succeed(
        &mut self,
        checks: Vec<PushTriggerCheck>,
        now_unix: u64,
    ) -> Result<(), DomainError> {
        self.finish(
            PushTriggerEvaluationState::Succeeded,
            None,
            checks,
            now_unix,
        )
    }

    pub fn configuration_error(
        &mut self,
        message: impl Into<String>,
        now_unix: u64,
    ) -> Result<(), DomainError> {
        self.finish(
            PushTriggerEvaluationState::ConfigurationError,
            Some(message.into()),
            Vec::new(),
            now_unix,
        )
    }

    pub fn fail(&mut self, message: impl Into<String>, now_unix: u64) -> Result<(), DomainError> {
        self.finish(
            PushTriggerEvaluationState::Failed,
            Some(message.into()),
            Vec::new(),
            now_unix,
        )
    }

    fn finish(
        &mut self,
        state: PushTriggerEvaluationState,
        message: Option<String>,
        checks: Vec<PushTriggerCheck>,
        now_unix: u64,
    ) -> Result<(), DomainError> {
        if self.state != PushTriggerEvaluationState::Pending {
            return Err(DomainError::conflict(
                "push trigger evaluation is already complete",
            ));
        }
        if now_unix < self.created_at_unix {
            return Err(DomainError::invalid_input(
                "push trigger evaluation completion cannot predate creation",
            ));
        }
        if matches!(
            state,
            PushTriggerEvaluationState::ConfigurationError | PushTriggerEvaluationState::Failed
        ) && message
            .as_deref()
            .is_none_or(|message| message.trim().is_empty())
        {
            return Err(DomainError::invalid_input(
                "failed push trigger evaluation requires a message",
            ));
        }
        self.state = state;
        self.message = message;
        self.checks = checks;
        self.completed_at_unix = Some(now_unix);
        Ok(())
    }
}
