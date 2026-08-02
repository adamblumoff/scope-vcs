use super::{
    Request, RequestActorRole, RequestEvent, RequestEventKind, RequestEventPayload,
    RequestReviewExitReason, RequestState, validate_required_id,
};
use crate::error::DomainError;

pub const PUBLIC_READY_REQUEST_LIMIT: usize = 3;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RequestReviewMutation {
    pub request: Request,
    pub events: Vec<RequestEvent>,
}

#[derive(Clone, Debug)]
pub struct MarkRequestReadyInput {
    pub request_id: String,
    pub actor_user_id: String,
    pub actor_is_author: bool,
    pub actor_can_mutate: bool,
    pub public_ready_count: usize,
    pub event_id: String,
    pub now_unix: u64,
}

#[derive(Clone, Debug)]
pub struct ReturnRequestToWorkingInput {
    pub request_id: String,
    pub actor_user_id: String,
    pub actor_is_author: bool,
    pub actor_can_mutate: bool,
    pub reason: RequestReviewExitReason,
    pub event_id: String,
    pub now_unix: u64,
}

#[derive(Clone, Debug)]
pub struct MergeRequestInput {
    pub request_id: String,
    pub actor_user_id: String,
    pub actor_is_maintainer: bool,
    pub merged_head_oid: String,
    pub merged_main_oid: String,
    pub merged_event_id: String,
    pub now_unix: u64,
}

pub fn mark_request_ready(
    request: &Request,
    input: MarkRequestReadyInput,
) -> Result<RequestReviewMutation, DomainError> {
    validate_command(request, &input.request_id, &input.actor_user_id)?;
    validate_required_id("request event id", &input.event_id)?;
    if !input.actor_is_author || request.author_user_id != input.actor_user_id {
        return Err(DomainError::forbidden(
            "only the request author can mark it ready for review",
        ));
    }
    if !input.actor_can_mutate {
        return Err(DomainError::forbidden("request mutation access required"));
    }
    if request.state != RequestState::Working {
        return Err(DomainError::conflict(
            "only working requests can be marked ready for review",
        ));
    }
    if request.git_snapshot.is_none() {
        return Err(DomainError::conflict(
            "request branch must be pushed before review",
        ));
    }
    if request.author_role == RequestActorRole::Public
        && input.public_ready_count >= PUBLIC_READY_REQUEST_LIMIT
    {
        return Err(DomainError::conflict(format!(
            "public contributors may have at most {PUBLIC_READY_REQUEST_LIMIT} ready requests per repository"
        )));
    }

    let mut next = request.clone();
    next.state = RequestState::ReadyForReview;
    next.first_ready_at_unix.get_or_insert(input.now_unix);
    next.ready_at_unix = Some(input.now_unix);
    next.updated_at_unix = input.now_unix;
    let event = append_event(
        &mut next,
        input.event_id,
        input.actor_user_id,
        RequestEventKind::ReadyForReview,
        RequestEventPayload::ReadyForReview {
            head_oid: request.head_oid.clone(),
        },
        input.now_unix,
    )?;
    next.validate_facts()?;
    Ok(mutation(next, vec![event]))
}

pub fn return_request_to_working(
    request: &Request,
    input: ReturnRequestToWorkingInput,
) -> Result<RequestReviewMutation, DomainError> {
    validate_command(request, &input.request_id, &input.actor_user_id)?;
    validate_required_id("request event id", &input.event_id)?;
    if request.state != RequestState::ReadyForReview {
        return Err(DomainError::conflict(
            "only ready requests can return to working",
        ));
    }
    authorize_exit(request, &input)?;

    let mut next = request.clone();
    next.state = RequestState::Working;
    next.ready_at_unix = None;
    next.updated_at_unix = input.now_unix;
    let event = append_event(
        &mut next,
        input.event_id,
        input.actor_user_id,
        RequestEventKind::ReturnedToWorking,
        RequestEventPayload::ReturnedToWorking {
            head_oid: request.head_oid.clone(),
            reason: input.reason,
        },
        input.now_unix,
    )?;
    next.validate_facts()?;
    Ok(mutation(next, vec![event]))
}

pub fn merge_request(
    request: &Request,
    input: MergeRequestInput,
) -> Result<RequestReviewMutation, DomainError> {
    validate_command(request, &input.request_id, &input.actor_user_id)?;
    validate_required_id("merged event id", &input.merged_event_id)?;
    validate_required_id("merged head oid", &input.merged_head_oid)?;
    validate_required_id("merged main oid", &input.merged_main_oid)?;
    if !input.actor_is_maintainer {
        return Err(DomainError::forbidden("repo maintainer required"));
    }
    if request.state != RequestState::ReadyForReview {
        return Err(DomainError::conflict("only ready requests can be merged"));
    }
    if input.merged_head_oid != request.head_oid {
        return Err(DomainError::conflict(
            "request branch changed before merge completed",
        ));
    }

    let mut next = request.clone();
    next.state = RequestState::Completed;
    next.ready_at_unix = None;
    next.completed_at_unix = Some(input.now_unix);
    next.completed_by_user_id = Some(input.actor_user_id.clone());
    next.merged_at_unix = Some(input.now_unix);
    next.merged_by_user_id = Some(input.actor_user_id.clone());
    next.merged_head_oid = Some(input.merged_head_oid.clone());
    next.merged_main_oid = Some(input.merged_main_oid.clone());
    next.updated_at_unix = input.now_unix;
    let event = append_event(
        &mut next,
        input.merged_event_id,
        input.actor_user_id,
        RequestEventKind::Merged,
        RequestEventPayload::Merged {
            head_oid: input.merged_head_oid,
            main_oid: input.merged_main_oid,
        },
        input.now_unix,
    )?;
    next.validate_facts()?;
    Ok(mutation(next, vec![event]))
}

fn authorize_exit(
    request: &Request,
    input: &ReturnRequestToWorkingInput,
) -> Result<(), DomainError> {
    match input.reason {
        RequestReviewExitReason::AuthorReturned => {
            if !input.actor_is_author || request.author_user_id != input.actor_user_id {
                return Err(DomainError::forbidden(
                    "only the request author can return it to working",
                ));
            }
            if !input.actor_can_mutate {
                return Err(DomainError::forbidden("request mutation access required"));
            }
        }
        RequestReviewExitReason::RevisionPushed | RequestReviewExitReason::ContentEdited => {
            if !input.actor_can_mutate {
                return Err(DomainError::forbidden("request mutation access required"));
            }
        }
    }
    Ok(())
}

fn validate_command(request: &Request, request_id: &str, actor: &str) -> Result<(), DomainError> {
    validate_required_id("request id", request_id)?;
    validate_required_id("actor user id", actor)?;
    if request.id != request_id {
        Err(DomainError::not_found("request not found"))
    } else {
        Ok(())
    }
}

fn append_event(
    request: &mut Request,
    id: String,
    actor: String,
    kind: RequestEventKind,
    payload: RequestEventPayload,
    now: u64,
) -> Result<RequestEvent, DomainError> {
    request.activity_version = request
        .activity_version
        .checked_add(1)
        .ok_or_else(|| DomainError::conflict("request activity version overflow"))?;
    Ok(RequestEvent {
        id,
        request_id: request.id.clone(),
        actor_user_id: actor,
        kind,
        position: request.activity_version,
        payload,
        created_at_unix: now,
    })
}

fn mutation(request: Request, events: Vec<RequestEvent>) -> RequestReviewMutation {
    RequestReviewMutation { request, events }
}
