use super::{
    Request, RequestActorRole, RequestAssessmentOutcome, RequestEvent, RequestEventKind,
    RequestEventPayload, RequestReviewExitReason, RequestState, validate_assessment_body,
    validate_required_id,
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
    pub actor_is_maintainer: bool,
    pub actor_can_mutate: bool,
    pub reason: RequestReviewExitReason,
    pub event_id: String,
    pub now_unix: u64,
}

#[derive(Clone, Debug)]
pub struct SetRequestHoldInput {
    pub request_id: String,
    pub actor_user_id: String,
    pub actor_is_maintainer: bool,
    pub held: bool,
    pub event_id: String,
    pub now_unix: u64,
}

#[derive(Clone, Debug)]
pub struct AssessRequestInput {
    pub request_id: String,
    pub actor_user_id: String,
    pub actor_is_maintainer: bool,
    pub outcome: RequestAssessmentOutcome,
    pub body_markdown: Option<String>,
    pub assessed_event_id: String,
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
    pub assessed_event_id: String,
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
    clear_hold(&mut next);
    next.updated_at_unix = input.now_unix;
    let payload = RequestEventPayload::ReadyForReview {
        head_oid: next.head_oid.clone(),
    };
    let event = append_event(
        &mut next,
        input.event_id,
        input.actor_user_id,
        RequestEventKind::ReadyForReview,
        payload,
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
    clear_hold(&mut next);
    next.updated_at_unix = input.now_unix;
    let payload = RequestEventPayload::ReturnedToWorking {
        head_oid: next.head_oid.clone(),
        reason: input.reason,
    };
    let event = append_event(
        &mut next,
        input.event_id,
        input.actor_user_id,
        RequestEventKind::ReturnedToWorking,
        payload,
        input.now_unix,
    )?;
    next.validate_facts()?;
    Ok(mutation(next, vec![event]))
}

pub fn set_request_hold(
    request: &Request,
    input: SetRequestHoldInput,
) -> Result<RequestReviewMutation, DomainError> {
    validate_command(request, &input.request_id, &input.actor_user_id)?;
    validate_required_id("request event id", &input.event_id)?;
    if !input.actor_is_maintainer {
        return Err(DomainError::forbidden("repo maintainer required"));
    }
    if request.state != RequestState::ReadyForReview {
        return Err(DomainError::conflict("only ready requests can be held"));
    }
    if request.held_at_unix.is_some() == input.held {
        return Ok(mutation(request.clone(), Vec::new()));
    }

    let mut next = request.clone();
    let (kind, payload) = if input.held {
        next.held_at_unix = Some(input.now_unix);
        next.held_by_user_id = Some(input.actor_user_id.clone());
        (
            RequestEventKind::Held,
            RequestEventPayload::Held {
                head_oid: next.head_oid.clone(),
            },
        )
    } else {
        clear_hold(&mut next);
        (
            RequestEventKind::HoldReleased,
            RequestEventPayload::HoldReleased {
                head_oid: next.head_oid.clone(),
            },
        )
    };
    next.updated_at_unix = input.now_unix;
    let event = append_event(
        &mut next,
        input.event_id,
        input.actor_user_id,
        kind,
        payload,
        input.now_unix,
    )?;
    next.validate_facts()?;
    Ok(mutation(next, vec![event]))
}

pub fn assess_request(
    request: &Request,
    input: AssessRequestInput,
) -> Result<RequestReviewMutation, DomainError> {
    validate_command(request, &input.request_id, &input.actor_user_id)?;
    validate_required_id("assessed event id", &input.assessed_event_id)?;
    if !input.actor_is_maintainer {
        return Err(DomainError::forbidden("repo maintainer required"));
    }
    if request.state != RequestState::ReadyForReview {
        return Err(DomainError::conflict("only ready requests can be assessed"));
    }
    validate_assessment_body(input.outcome, input.body_markdown.as_deref())?;

    let mut next = request.clone();
    next.state = RequestState::Completed;
    next.ready_at_unix = None;
    clear_hold(&mut next);
    next.assessment_outcome = Some(input.outcome);
    next.assessment_body_markdown = input.body_markdown.clone();
    next.assessed_at_unix = Some(input.now_unix);
    next.assessed_by_user_id = Some(input.actor_user_id.clone());
    next.completed_at_unix = Some(input.now_unix);
    next.completed_by_user_id = Some(input.actor_user_id.clone());
    next.updated_at_unix = input.now_unix;
    let payload = RequestEventPayload::Assessed {
        head_oid: next.head_oid.clone(),
        outcome: input.outcome,
        body_markdown: input.body_markdown,
    };
    let assessed = append_event(
        &mut next,
        input.assessed_event_id,
        input.actor_user_id,
        RequestEventKind::Assessed,
        payload,
        input.now_unix,
    )?;
    next.validate_facts()?;
    Ok(mutation(next, vec![assessed]))
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
    if request.merged_at_unix.is_some() {
        return Err(DomainError::conflict("request is already merged"));
    }
    if input.merged_head_oid != request.head_oid {
        return Err(DomainError::conflict(
            "request branch changed before merge completed",
        ));
    }

    let mut result = match request.state {
        RequestState::ReadyForReview => assess_request(
            request,
            AssessRequestInput {
                request_id: input.request_id.clone(),
                actor_user_id: input.actor_user_id.clone(),
                actor_is_maintainer: true,
                outcome: RequestAssessmentOutcome::Accepted,
                body_markdown: None,
                assessed_event_id: input.assessed_event_id,
                now_unix: input.now_unix,
            },
        )?,
        RequestState::Completed
            if request.assessment_outcome == Some(RequestAssessmentOutcome::Accepted) =>
        {
            mutation(request.clone(), Vec::new())
        }
        RequestState::Completed => {
            return Err(DomainError::conflict(
                "only accepted completed requests can be merged",
            ));
        }
        RequestState::Working => {
            return Err(DomainError::conflict(
                "only ready or accepted requests can be merged",
            ));
        }
    };

    result.request.merged_at_unix = Some(input.now_unix);
    result.request.merged_by_user_id = Some(input.actor_user_id.clone());
    result.request.merged_head_oid = Some(input.merged_head_oid.clone());
    result.request.merged_main_oid = Some(input.merged_main_oid.clone());
    result.request.updated_at_unix = input.now_unix;
    let payload = RequestEventPayload::Merged {
        head_oid: input.merged_head_oid,
        main_oid: input.merged_main_oid,
    };
    result.events.push(append_event(
        &mut result.request,
        input.merged_event_id,
        input.actor_user_id,
        RequestEventKind::Merged,
        payload,
        input.now_unix,
    )?);
    result.request.validate_facts()?;
    Ok(result)
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
            if request.held_at_unix.is_some() {
                return Err(DomainError::conflict(
                    "held request cannot be returned by its author",
                ));
            }
        }
        RequestReviewExitReason::ChangesRequested if !input.actor_is_maintainer => {
            return Err(DomainError::forbidden("repo maintainer required"));
        }
        RequestReviewExitReason::RevisionPushed | RequestReviewExitReason::ContentEdited => {
            if !input.actor_can_mutate {
                return Err(DomainError::forbidden("request mutation access required"));
            }
            if request.held_at_unix.is_some() && !input.actor_is_maintainer {
                return Err(DomainError::conflict(
                    "request cannot be changed while held",
                ));
            }
        }
        RequestReviewExitReason::ChangesRequested => {}
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
    validate_required_id("request event id", &id)?;
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

fn clear_hold(request: &mut Request) {
    request.held_at_unix = None;
    request.held_by_user_id = None;
}
