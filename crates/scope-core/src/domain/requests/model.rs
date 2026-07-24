use super::{
    REQUEST_MAX_STAKE_CREDITS, RequestAssessmentOutcome, RequestReviewExitReason,
    validate_assessment_body,
};
use crate::{domain::store::SourceBlob, error::ApiError};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(any(test, feature = "ts"), derive(ts_rs::TS))]
pub enum RequestActorRole {
    Public,
    Member,
    Owner,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(any(test, feature = "ts"), derive(ts_rs::TS))]
pub enum RequestAudience {
    Public,
    Private,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(any(test, feature = "ts"), derive(ts_rs::TS))]
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
    pub ready_queue_version: Option<u64>,
    pub current_stake_credits: u32,
    pub first_ready_at_unix: Option<u64>,
    pub ready_at_unix: Option<u64>,
    pub held_at_unix: Option<u64>,
    pub held_by_user_id: Option<String>,
    pub assessment_outcome: Option<RequestAssessmentOutcome>,
    pub assessment_body_markdown: Option<String>,
    pub assessed_at_unix: Option<u64>,
    pub assessed_by_user_id: Option<String>,
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

    pub fn validate_facts(&self) -> Result<(), ApiError> {
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
#[cfg_attr(any(test, feature = "ts"), derive(ts_rs::TS))]
pub enum RequestEventKind {
    Started,
    ReadyForReview,
    ReturnedToWorking,
    RevisionPushed,
    Held,
    HoldReleased,
    Assessed,
    Merged,
    Closed,
    Settled,
    DescriptionEdited,
    DiscussionResolved,
    DiscussionReopened,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(any(test, feature = "ts"), derive(ts_rs::TS))]
pub struct RequestDescriptionAuditFact {
    pub sha256: String,
    pub byte_count: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(any(test, feature = "ts"), derive(ts_rs::TS))]
pub enum RequestEventPayload {
    Started {
        title: String,
        description_markdown: String,
    },
    ReadyForReview {
        head_oid: String,
        stake_credits: u32,
    },
    ReturnedToWorking {
        head_oid: String,
        stake_credits: u32,
        reason: RequestReviewExitReason,
    },
    RevisionPushed {
        old_head_oid: String,
        new_head_oid: String,
        note: Option<String>,
    },
    Held {
        head_oid: String,
    },
    HoldReleased {
        head_oid: String,
    },
    Assessed {
        head_oid: String,
        outcome: RequestAssessmentOutcome,
        body_markdown: Option<String>,
        stake_credits: u32,
    },
    Merged {
        head_oid: String,
        main_oid: String,
    },
    Closed {
        head_oid: String,
    },
    Settled {
        settlement: super::RequestSettlement,
    },
    DescriptionEdited {
        before: RequestDescriptionAuditFact,
        after: RequestDescriptionAuditFact,
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

pub fn validate_request_facts(request: &Request) -> Result<(), ApiError> {
    if request.updated_at_unix < request.created_at_unix {
        return Err(ApiError::conflict(
            "request update time cannot precede creation time",
        ));
    }
    for (label, value) in [
        ("first ready time", request.first_ready_at_unix),
        ("ready time", request.ready_at_unix),
        ("hold time", request.held_at_unix),
        ("assessment time", request.assessed_at_unix),
        ("completion time", request.completed_at_unix),
        ("merge time", request.merged_at_unix),
    ] {
        if value
            .is_some_and(|value| value < request.created_at_unix || value > request.updated_at_unix)
        {
            return Err(ApiError::conflict(format!(
                "request {label} must be within its lifetime"
            )));
        }
    }
    if request.current_stake_credits > REQUEST_MAX_STAKE_CREDITS {
        return Err(ApiError::conflict(format!(
            "request current stake cannot exceed {REQUEST_MAX_STAKE_CREDITS} credits"
        )));
    }
    if request.is_published() != request.ready_queue_version.is_some() {
        return Err(ApiError::conflict(
            "published request and ready queue version must be set together",
        ));
    }
    match request.state {
        RequestState::Working => {
            require_none("working request ready time", request.ready_at_unix)?;
            require_none("working request hold time", request.held_at_unix)?;
            require_none_ref("working request holder", request.held_by_user_id.as_ref())?;
            require_zero(
                "working request current stake",
                request.current_stake_credits,
            )?;
            require_none("working request completion time", request.completed_at_unix)?;
        }
        RequestState::ReadyForReview => {
            require_some(
                "ready request publication time",
                request.first_ready_at_unix,
            )?;
            require_some("ready request ready time", request.ready_at_unix)?;
            require_none("ready request completion time", request.completed_at_unix)?;
            let hold_is_complete =
                request.held_at_unix.is_some() == request.held_by_user_id.is_some();
            if !hold_is_complete {
                return Err(ApiError::conflict(
                    "ready request hold time and holder must be set together",
                ));
            }
            if request
                .held_at_unix
                .zip(request.ready_at_unix)
                .is_some_and(|(held_at, ready_at)| held_at < ready_at)
            {
                return Err(ApiError::conflict(
                    "ready request hold time cannot precede ready time",
                ));
            }
            if request.author_role == RequestActorRole::Public {
                if request.current_stake_credits == 0 {
                    return Err(ApiError::conflict(
                        "public ready request requires a current stake",
                    ));
                }
            } else {
                require_zero(
                    "maintainer ready request current stake",
                    request.current_stake_credits,
                )?;
            }
        }
        RequestState::Completed => {
            require_some(
                "completed request publication time",
                request.first_ready_at_unix,
            )?;
            require_none("completed request ready time", request.ready_at_unix)?;
            require_none("completed request hold time", request.held_at_unix)?;
            require_none_ref("completed request holder", request.held_by_user_id.as_ref())?;
            require_zero(
                "completed request current stake",
                request.current_stake_credits,
            )?;
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

    let assessment_count = [
        request.assessment_outcome.is_some(),
        request.assessed_at_unix.is_some(),
        request.assessed_by_user_id.is_some(),
    ]
    .into_iter()
    .filter(|present| *present)
    .count();
    if assessment_count != 0 && assessment_count != 3 {
        return Err(ApiError::conflict(
            "assessment outcome, time, and actor must be set together",
        ));
    }
    if assessment_count > 0 && request.state != RequestState::Completed {
        return Err(ApiError::conflict(
            "only completed requests may be assessed",
        ));
    }
    if let Some(assessed_at_unix) = request.assessed_at_unix
        && Some(assessed_at_unix) != request.completed_at_unix
    {
        return Err(ApiError::conflict(
            "assessment and completion must happen atomically",
        ));
    }
    if let Some(outcome) = request.assessment_outcome {
        validate_assessment_body(outcome, request.assessment_body_markdown.as_deref())
            .map_err(|_| ApiError::conflict("rejected assessment requires a written reason"))?;
    }
    if request.assessment_outcome.is_none() && request.assessment_body_markdown.is_some() {
        return Err(ApiError::conflict(
            "assessment body requires an assessment outcome",
        ));
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
        return Err(ApiError::conflict(
            "merge time, actor, head, and main oid must be set together",
        ));
    }
    if merge_count > 0
        && (request.state != RequestState::Completed
            || request.assessment_outcome != Some(RequestAssessmentOutcome::Accepted))
    {
        return Err(ApiError::conflict(
            "merged requests must be completed and accepted",
        ));
    }
    if let (Some(completed_at_unix), Some(merged_at_unix)) =
        (request.completed_at_unix, request.merged_at_unix)
        && merged_at_unix < completed_at_unix
    {
        return Err(ApiError::conflict(
            "request merge cannot precede completion",
        ));
    }
    if let (Some(first_ready_at_unix), Some(ready_at_unix)) =
        (request.first_ready_at_unix, request.ready_at_unix)
        && ready_at_unix < first_ready_at_unix
    {
        return Err(ApiError::conflict(
            "current ready time cannot precede first ready time",
        ));
    }
    if let (Some(first_ready_at_unix), Some(completed_at_unix)) =
        (request.first_ready_at_unix, request.completed_at_unix)
        && completed_at_unix < first_ready_at_unix
    {
        return Err(ApiError::conflict(
            "request completion cannot precede first publication",
        ));
    }

    Ok(())
}

fn require_some(label: &str, value: Option<u64>) -> Result<(), ApiError> {
    if value.is_none() {
        Err(ApiError::conflict(format!("{label} is required")))
    } else {
        Ok(())
    }
}

fn require_some_ref<T>(label: &str, value: Option<&T>) -> Result<(), ApiError> {
    if value.is_none() {
        Err(ApiError::conflict(format!("{label} is required")))
    } else {
        Ok(())
    }
}

fn require_none(label: &str, value: Option<u64>) -> Result<(), ApiError> {
    if value.is_some() {
        Err(ApiError::conflict(format!("{label} must be empty")))
    } else {
        Ok(())
    }
}

fn require_none_ref<T>(label: &str, value: Option<&T>) -> Result<(), ApiError> {
    if value.is_some() {
        Err(ApiError::conflict(format!("{label} must be empty")))
    } else {
        Ok(())
    }
}

fn require_zero(label: &str, value: u32) -> Result<(), ApiError> {
    if value == 0 {
        Ok(())
    } else {
        Err(ApiError::conflict(format!("{label} must be zero")))
    }
}
