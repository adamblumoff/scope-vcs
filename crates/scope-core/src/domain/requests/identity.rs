use super::{
    REQUEST_DESCRIPTION_MAX_BYTES, REQUEST_TITLE_MAX_BYTES, Request, RequestEvent,
    RequestEventKind, RequestEventPayload, RequestIdentityAuditFact, RequestState,
    RequestTimelineMutation, advance_request_activity, ensure_event_id_available, open_request_mut,
    validate_body_size, validate_required_id,
};
use crate::error::ApiError;
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;

#[derive(Clone, Debug)]
pub struct EditRequestIdentityInput {
    pub request_id: String,
    pub actor_user_id: String,
    pub actor_can_edit_identity: bool,
    pub event_id: String,
    pub title: Option<String>,
    pub description_markdown: Option<String>,
    pub now_unix: u64,
}

pub fn edit_request_identity(
    requests: &mut BTreeMap<String, Request>,
    events: &mut BTreeMap<String, RequestEvent>,
    input: EditRequestIdentityInput,
) -> Result<RequestTimelineMutation, ApiError> {
    validate_required_id("request id", &input.request_id)?;
    validate_required_id("actor user id", &input.actor_user_id)?;
    validate_required_id("event id", &input.event_id)?;
    ensure_event_id_available(events, &input.event_id)?;
    if !input.actor_can_edit_identity {
        return Err(ApiError::forbidden("request edit access required"));
    }
    if input.title.is_none() && input.description_markdown.is_none() {
        return Err(ApiError::bad_request(
            "request edit requires a title or description",
        ));
    }
    if let Some(title) = &input.title {
        validate_required_id("request title", title)?;
        validate_body_size("request title", title, REQUEST_TITLE_MAX_BYTES)?;
    }
    if let Some(description) = &input.description_markdown {
        validate_body_size(
            "request description",
            description,
            REQUEST_DESCRIPTION_MAX_BYTES,
        )?;
    }
    let request = open_request_mut(requests, &input.request_id)?;
    if request.held_at_unix.is_some() {
        return Err(ApiError::conflict("request cannot be edited while held"));
    }
    if request.state == RequestState::ReadyForReview {
        return Err(ApiError::conflict(
            "request cannot be edited while ready for review",
        ));
    }
    let title = input.title.unwrap_or_else(|| request.title.clone());
    let description_markdown = input
        .description_markdown
        .unwrap_or_else(|| request.description_markdown.clone());
    if request.title == title && request.description_markdown == description_markdown {
        return Err(ApiError::conflict(
            "request title and description are unchanged",
        ));
    }
    let before = identity_audit_fact(&request.title, &request.description_markdown)?;
    let after = identity_audit_fact(&title, &description_markdown)?;
    request.title = title;
    request.description_markdown = description_markdown;
    request.updated_at_unix = input.now_unix;
    let position = advance_request_activity(request)?;
    let request = request.clone();
    let event = RequestEvent {
        id: input.event_id,
        request_id: request.id.clone(),
        actor_user_id: input.actor_user_id,
        kind: RequestEventKind::IdentityEdited,
        position,
        payload: RequestEventPayload::IdentityEdited { before, after },
        created_at_unix: input.now_unix,
    };
    events.insert(event.id.clone(), event.clone());
    Ok(RequestTimelineMutation { request, event })
}

fn identity_audit_fact(
    title: &str,
    description_markdown: &str,
) -> Result<RequestIdentityAuditFact, ApiError> {
    Ok(RequestIdentityAuditFact {
        title_sha256: hex::encode(Sha256::digest(title.as_bytes())),
        title_byte_count: u64::try_from(title.len()).map_err(ApiError::internal)?,
        description_sha256: hex::encode(Sha256::digest(description_markdown.as_bytes())),
        description_byte_count: u64::try_from(description_markdown.len())
            .map_err(ApiError::internal)?,
    })
}
