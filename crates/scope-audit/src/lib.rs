use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum AuditAction {
    ReadPath,
    WritePath,
    CreateManifest,
    PublishSnapshot,
    ProjectGitView,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum AuditResult {
    Allowed,
    Denied,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuditEvent {
    pub id: Uuid,
    pub actor_id: String,
    pub repo_id: String,
    pub action: AuditAction,
    pub object_path: Option<String>,
    pub revision_id: Option<String>,
    pub result: AuditResult,
    pub reason: Option<String>,
}

impl AuditEvent {
    pub fn allowed(
        actor_id: impl Into<String>,
        repo_id: impl Into<String>,
        action: AuditAction,
    ) -> Self {
        Self {
            id: Uuid::now_v7(),
            actor_id: actor_id.into(),
            repo_id: repo_id.into(),
            action,
            object_path: None,
            revision_id: None,
            result: AuditResult::Allowed,
            reason: None,
        }
    }
}
