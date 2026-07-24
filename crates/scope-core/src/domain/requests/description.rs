use super::{
    REQUEST_DESCRIPTION_MAX_BYTES, Request, RequestDescriptionAuditFact, RequestEvent,
    RequestEventKind, RequestEventPayload, RequestState, RequestTimelineMutation,
    advance_request_activity, ensure_event_id_available, open_request_mut, validate_body_size,
    validate_required_id,
};
use crate::error::ApiError;
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;

#[derive(Clone, Debug)]
pub struct UpdateRequestDescriptionInput {
    pub request_id: String,
    pub actor_user_id: String,
    pub actor_can_edit_description: bool,
    pub event_id: String,
    pub description_markdown: String,
    pub now_unix: u64,
}

pub fn update_request_description(
    requests: &mut BTreeMap<String, Request>,
    events: &mut BTreeMap<String, RequestEvent>,
    input: UpdateRequestDescriptionInput,
) -> Result<RequestTimelineMutation, ApiError> {
    validate_required_id("request id", &input.request_id)?;
    validate_required_id("actor user id", &input.actor_user_id)?;
    validate_required_id("event id", &input.event_id)?;
    ensure_event_id_available(events, &input.event_id)?;
    if !input.actor_can_edit_description {
        return Err(ApiError::forbidden(
            "request description edit access required",
        ));
    }
    let request = open_request_mut(requests, &input.request_id)?;
    if request.held_at_unix.is_some() {
        return Err(ApiError::conflict(
            "request description cannot be edited while held",
        ));
    }
    if request.state == RequestState::ReadyForReview {
        return Err(ApiError::conflict(
            "request description cannot be edited while ready for review",
        ));
    }
    if request.description_markdown == input.description_markdown {
        return Err(ApiError::conflict("request description is unchanged"));
    }
    validate_body_size(
        "request description",
        &input.description_markdown,
        REQUEST_DESCRIPTION_MAX_BYTES,
    )?;
    let before = description_audit_fact(&request.description_markdown)?;
    let after = description_audit_fact(&input.description_markdown)?;
    request.description_markdown = input.description_markdown;
    request.updated_at_unix = input.now_unix;
    let position = advance_request_activity(request)?;
    let request = request.clone();
    let event = RequestEvent {
        id: input.event_id,
        request_id: request.id.clone(),
        actor_user_id: input.actor_user_id,
        kind: RequestEventKind::DescriptionEdited,
        position,
        payload: RequestEventPayload::DescriptionEdited { before, after },
        created_at_unix: input.now_unix,
    };
    events.insert(event.id.clone(), event.clone());
    Ok(RequestTimelineMutation { request, event })
}

fn description_audit_fact(value: &str) -> Result<RequestDescriptionAuditFact, ApiError> {
    Ok(RequestDescriptionAuditFact {
        sha256: hex::encode(Sha256::digest(value.as_bytes())),
        byte_count: u64::try_from(value.len()).map_err(ApiError::internal)?,
    })
}
