//! One-way request submission and terminal merge behavior.

use super::{
    Request, RequestEvent, RequestEventKind, RequestEventPayload, RequestState,
    validate_required_id,
};
use crate::error::DomainError;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RequestLifecycleMutation {
    pub request: Request,
    pub events: Vec<RequestEvent>,
}

#[derive(Clone, Debug)]
pub struct SubmitRequestInput {
    pub request_id: String,
    pub actor_user_id: String,
    pub actor_is_author: bool,
    pub actor_can_submit: bool,
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

pub fn submit_request(
    request: &Request,
    input: SubmitRequestInput,
) -> Result<RequestLifecycleMutation, DomainError> {
    validate_command(request, &input.request_id, &input.actor_user_id)?;
    validate_required_id("request event id", &input.event_id)?;
    if !input.actor_is_author || request.author_user_id != input.actor_user_id {
        return Err(DomainError::forbidden(
            "only the request author can submit it",
        ));
    }
    if request.state() != RequestState::Draft {
        return Err(DomainError::conflict("request has already been submitted"));
    }
    if !input.actor_can_submit {
        return Err(DomainError::forbidden("request submission access required"));
    }
    if request.git_snapshot.is_none() {
        return Err(DomainError::conflict(
            "request branch must be pushed before submission",
        ));
    }

    let mut next = request.clone();
    next.submitted_at_unix = Some(input.now_unix);
    next.updated_at_unix = input.now_unix;
    let event = append_event(
        &mut next,
        input.event_id,
        input.actor_user_id,
        RequestEventKind::Submitted,
        RequestEventPayload::Submitted {
            head_oid: request.head_oid.clone(),
        },
        input.now_unix,
    )?;
    next.validate_facts()?;
    Ok(RequestLifecycleMutation {
        request: next,
        events: vec![event],
    })
}

pub fn merge_request(
    request: &Request,
    input: MergeRequestInput,
) -> Result<RequestLifecycleMutation, DomainError> {
    validate_command(request, &input.request_id, &input.actor_user_id)?;
    validate_required_id("merged event id", &input.merged_event_id)?;
    validate_required_id("merged head oid", &input.merged_head_oid)?;
    validate_required_id("merged main oid", &input.merged_main_oid)?;
    if !input.actor_is_maintainer {
        return Err(DomainError::forbidden("repo maintainer required"));
    }
    if request.state() != RequestState::Open {
        return Err(DomainError::conflict("only open requests can be merged"));
    }
    if input.merged_head_oid != request.head_oid {
        return Err(DomainError::conflict(
            "request branch changed before merge completed",
        ));
    }

    let mut next = request.clone();
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
    Ok(RequestLifecycleMutation {
        request: next,
        events: vec![event],
    })
}

fn validate_command(request: &Request, request_id: &str, actor: &str) -> Result<(), DomainError> {
    validate_required_id("request id", request_id)?;
    validate_required_id("actor user id", actor)?;
    if request.id == request_id {
        Ok(())
    } else {
        Err(DomainError::not_found("request not found"))
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
