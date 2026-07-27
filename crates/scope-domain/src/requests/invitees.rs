use super::{Request, RequestAudience, RequestInvitee, RequestState, validate_required_id};
use crate::error::DomainError;
use std::collections::BTreeMap;

pub const REQUEST_ACTIVE_INVITEE_LIMIT: usize = 30;

#[derive(Clone, Debug)]
pub struct AddRequestInviteeInput {
    pub actor_user_id: String,
    pub target_user_id: String,
    pub actor_can_manage_invitees: bool,
    pub target_is_maintainer: bool,
    pub now_unix: u64,
}

#[derive(Clone, Debug)]
pub struct RemoveRequestInviteeInput {
    pub actor_user_id: String,
    pub target_user_id: String,
    pub actor_can_manage_invitees: bool,
}

#[derive(Clone, Debug)]
pub struct LeaveRequestInput {
    pub actor_user_id: String,
    pub actor_can_leave_request: bool,
}

pub fn add_request_invitee(
    request: &Request,
    invitees: &mut BTreeMap<String, RequestInvitee>,
    input: AddRequestInviteeInput,
) -> Result<RequestInvitee, DomainError> {
    validate_invitee_request(request)?;
    validate_required_id("actor user id", &input.actor_user_id)?;
    validate_required_id("invitee user id", &input.target_user_id)?;
    if !input.actor_can_manage_invitees {
        return Err(DomainError::forbidden(
            "request invite management access required",
        ));
    }
    if input.target_user_id == request.author_user_id {
        return Err(DomainError::conflict("request author cannot be an invitee"));
    }
    if input.target_is_maintainer {
        return Err(DomainError::conflict(
            "repo maintainers do not need request invitations",
        ));
    }
    if invitees.contains_key(&input.target_user_id) {
        return Err(DomainError::conflict(
            "user is already invited to this request",
        ));
    }
    if invitees.len() >= REQUEST_ACTIVE_INVITEE_LIMIT {
        return Err(DomainError::conflict(format!(
            "request cannot have more than {REQUEST_ACTIVE_INVITEE_LIMIT} active invitees"
        )));
    }
    let invitee = RequestInvitee {
        request_id: request.id.clone(),
        user_id: input.target_user_id,
        invited_by_user_id: input.actor_user_id,
        created_at_unix: input.now_unix,
    };
    invitees.insert(invitee.user_id.clone(), invitee.clone());
    Ok(invitee)
}

pub fn remove_request_invitee(
    request: &Request,
    invitees: &mut BTreeMap<String, RequestInvitee>,
    input: RemoveRequestInviteeInput,
) -> Result<RequestInvitee, DomainError> {
    validate_invitee_request(request)?;
    validate_required_id("actor user id", &input.actor_user_id)?;
    validate_required_id("invitee user id", &input.target_user_id)?;
    if !input.actor_can_manage_invitees {
        return Err(DomainError::forbidden(
            "request invite management access required",
        ));
    }
    invitees
        .remove(&input.target_user_id)
        .ok_or_else(|| DomainError::not_found("request invitee not found"))
}

pub fn leave_request(
    request: &Request,
    invitees: &mut BTreeMap<String, RequestInvitee>,
    input: LeaveRequestInput,
) -> Result<RequestInvitee, DomainError> {
    validate_invitee_request(request)?;
    validate_required_id("actor user id", &input.actor_user_id)?;
    if !input.actor_can_leave_request {
        return Err(DomainError::forbidden("request leave access required"));
    }
    invitees
        .remove(&input.actor_user_id)
        .ok_or_else(|| DomainError::not_found("request invitee not found"))
}

fn validate_invitee_request(request: &Request) -> Result<(), DomainError> {
    if request.audience != RequestAudience::Public {
        return Err(DomainError::conflict(
            "private requests do not support invitees",
        ));
    }
    if request.state == RequestState::Completed {
        return Err(DomainError::conflict("request is completed"));
    }
    Ok(())
}
