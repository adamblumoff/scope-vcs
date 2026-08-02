use super::RequestReviewExitReason;
use crate::{error::DomainError, store::SourceBlob};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum RequestActorRole {
    Public,
    Member,
    Owner,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum RequestAudience {
    Public,
    Private,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum RequestState {
    Working,
    ReadyForReview,
    Completed,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Request {
    pub id: String,
    pub repo_id: String,
    pub name: String,
    pub author_user_id: String,
    pub author_role: RequestActorRole,
    pub audience: RequestAudience,
    pub base_main_oid: String,
    pub head_oid: String,
    pub git_snapshot: Option<SourceBlob>,
    pub title: String,
    pub description_markdown: String,
    pub state: RequestState,
    pub activity_version: u64,
    pub first_ready_at_unix: Option<u64>,
    pub ready_at_unix: Option<u64>,
    pub completed_at_unix: Option<u64>,
    pub completed_by_user_id: Option<String>,
    pub merged_at_unix: Option<u64>,
    pub merged_by_user_id: Option<String>,
    pub merged_head_oid: Option<String>,
    pub merged_main_oid: Option<String>,
    pub created_at_unix: u64,
    pub updated_at_unix: u64,
}

impl Request {
    pub fn is_published(&self) -> bool {
        self.first_ready_at_unix.is_some()
    }

    pub fn validate_facts(&self) -> Result<(), DomainError> {
        validate_request_facts(self)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RequestInvitee {
    pub request_id: String,
    pub user_id: String,
    pub invited_by_user_id: String,
    pub created_at_unix: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum RequestEventKind {
    Started,
    ReadyForReview,
    ReturnedToWorking,
    RevisionPushed,
    Merged,
    Closed,
    IdentityEdited,
    DiscussionResolved,
    DiscussionReopened,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RequestIdentityAuditFact {
    pub title_sha256: String,
    pub title_byte_count: u64,
    pub description_sha256: String,
    pub description_byte_count: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum RequestEventPayload {
    Started {
        title: String,
        description_markdown: String,
    },
    ReadyForReview {
        head_oid: String,
    },
    ReturnedToWorking {
        head_oid: String,
        reason: RequestReviewExitReason,
    },
    RevisionPushed {
        old_head_oid: String,
        new_head_oid: String,
        note: Option<String>,
    },
    Merged {
        head_oid: String,
        main_oid: String,
    },
    Closed {
        head_oid: String,
    },
    IdentityEdited {
        before: RequestIdentityAuditFact,
        after: RequestIdentityAuditFact,
    },
    DiscussionResolved {
        discussion_id: String,
    },
    DiscussionReopened {
        discussion_id: String,
    },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RequestEvent {
    pub id: String,
    pub request_id: String,
    pub actor_user_id: String,
    pub kind: RequestEventKind,
    pub position: u64,
    pub payload: RequestEventPayload,
    pub created_at_unix: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RequestTimelineMutation {
    pub request: Request,
    pub event: RequestEvent,
}

pub fn validate_request_facts(request: &Request) -> Result<(), DomainError> {
    if request.updated_at_unix < request.created_at_unix {
        return Err(DomainError::conflict(
            "request update time cannot precede creation time",
        ));
    }
    for (label, value) in [
        ("first ready time", request.first_ready_at_unix),
        ("ready time", request.ready_at_unix),
        ("completion time", request.completed_at_unix),
        ("merge time", request.merged_at_unix),
    ] {
        if value
            .is_some_and(|value| value < request.created_at_unix || value > request.updated_at_unix)
        {
            return Err(DomainError::conflict(format!(
                "request {label} must be within its lifetime"
            )));
        }
    }
    match request.state {
        RequestState::Working => {
            require_none("working request ready time", request.ready_at_unix)?;
            require_none("working request completion time", request.completed_at_unix)?;
        }
        RequestState::ReadyForReview => {
            require_some(
                "ready request publication time",
                request.first_ready_at_unix,
            )?;
            require_some("ready request ready time", request.ready_at_unix)?;
            require_none("ready request completion time", request.completed_at_unix)?;
        }
        RequestState::Completed => {
            require_some(
                "completed request publication time",
                request.first_ready_at_unix,
            )?;
            require_none("completed request ready time", request.ready_at_unix)?;
            require_some(
                "completed request completion time",
                request.completed_at_unix,
            )?;
            require_some_ref(
                "completed request completion actor",
                request.completed_by_user_id.as_ref(),
            )?;
        }
    }

    let merge_count = [
        request.merged_at_unix.is_some(),
        request.merged_by_user_id.is_some(),
        request.merged_head_oid.is_some(),
        request.merged_main_oid.is_some(),
    ]
    .into_iter()
    .filter(|present| *present)
    .count();
    if merge_count != 0 && merge_count != 4 {
        return Err(DomainError::conflict(
            "merge time, actor, head, and main oid must be set together",
        ));
    }
    if merge_count > 0 && request.state != RequestState::Completed {
        return Err(DomainError::conflict("merged requests must be completed"));
    }
    if let (Some(completed_at_unix), Some(merged_at_unix)) =
        (request.completed_at_unix, request.merged_at_unix)
        && merged_at_unix < completed_at_unix
    {
        return Err(DomainError::conflict(
            "request merge cannot precede completion",
        ));
    }
    if let (Some(first_ready_at_unix), Some(ready_at_unix)) =
        (request.first_ready_at_unix, request.ready_at_unix)
        && ready_at_unix < first_ready_at_unix
    {
        return Err(DomainError::conflict(
            "current ready time cannot precede first ready time",
        ));
    }
    if let (Some(first_ready_at_unix), Some(completed_at_unix)) =
        (request.first_ready_at_unix, request.completed_at_unix)
        && completed_at_unix < first_ready_at_unix
    {
        return Err(DomainError::conflict(
            "request completion cannot precede first publication",
        ));
    }

    Ok(())
}

fn require_some(label: &str, value: Option<u64>) -> Result<(), DomainError> {
    if value.is_none() {
        Err(DomainError::conflict(format!("{label} is required")))
    } else {
        Ok(())
    }
}

fn require_some_ref<T>(label: &str, value: Option<&T>) -> Result<(), DomainError> {
    if value.is_none() {
        Err(DomainError::conflict(format!("{label} is required")))
    } else {
        Ok(())
    }
}

fn require_none(label: &str, value: Option<u64>) -> Result<(), DomainError> {
    if value.is_some() {
        Err(DomainError::conflict(format!("{label} must be empty")))
    } else {
        Ok(())
    }
}
