use crate::error::DomainError;
use std::collections::BTreeMap;

mod access;
pub use access::{
    RequestListPredicate, RequestMergeability, RequestMergeabilityStatus, RequestPermissions,
    RequestPolicyDecision, RequestViewer, request_actor_role, request_list_mergeability,
    request_list_predicate, request_mergeability, request_policy,
};
mod revisions;
pub use revisions::{RequestRevision, select_request_review_revision};
mod identity;
pub use identity::{EditRequestIdentityInput, edit_request_identity, request_identity_audit_fact};
mod discussions;
pub use discussions::{
    CreateRequestDiscussionInput, CreateRequestDiscussionMutation,
    CreateRequestDiscussionReplyInput, CreateRequestDiscussionReplyMutation,
    MarkRequestDiscussionReadInput, ReopenAndReplyToRequestDiscussionInput,
    ReopenRequestDiscussionInput, RequestDiscussion, RequestDiscussionAnchor,
    RequestDiscussionMutation, RequestDiscussionReadState, RequestDiscussionReply,
    RequestDiscussionStatus, ResolveRequestDiscussionInput, create_request_discussion,
    create_request_discussion_reply, ensure_request_discussion_transition_allowed,
    mark_request_discussion_read, reopen_and_reply_to_request_discussion,
    reopen_request_discussion, resolve_request_discussion,
};
mod lifecycle;
pub use lifecycle::{
    CloseRequestInput, CloseRequestMutation, RecordRequestRevisionInput,
    RecordWorkingRequestUploadInput, RequestRevisionMutation, StartRequestInput,
    StartRequestMutation, WorkingRequestUploadMutation, close_request, record_request_revision,
    record_working_request_upload, start_request, validate_request_name,
};
mod invitees;
pub use invitees::{
    AddRequestInviteeInput, LeaveRequestInput, REQUEST_ACTIVE_INVITEE_LIMIT,
    RemoveRequestInviteeInput, add_request_invitee, leave_request, remove_request_invitee,
};
mod limits;
pub use limits::{
    PUBLIC_WORKING_REQUEST_LIMIT, REQUEST_ACTIVITY_PAGE_MAX_EVENTS, REQUEST_DESCRIPTION_MAX_BYTES,
    REQUEST_DISCUSSION_BODY_MAX_BYTES, REQUEST_DISCUSSION_CLIENT_ID_MAX_BYTES,
    REQUEST_LIST_DEFAULT_PAGE_SIZE, REQUEST_LIST_MAX_PAGE_SIZE, REQUEST_TIMELINE_BODY_MAX_BYTES,
    REQUEST_TITLE_MAX_BYTES,
};
pub(super) use limits::{validate_body_size, validate_required_body};
mod model;
pub use model::{
    Request, RequestActorRole, RequestAudience, RequestEvent, RequestEventKind,
    RequestEventPayload, RequestIdentityAuditFact, RequestInvitee, RequestState,
    RequestTimelineMutation, validate_request_facts,
};
mod queue;
pub use queue::RequestQueueSection;
mod ratings;
pub use ratings::{
    CreateRequestRatingInput, REQUEST_RATING_REASON_MAX_BYTES, RequestRating, RequestReputation,
    create_request_rating, eligible_rating_subject_user_id,
};
mod submission;
pub use submission::{
    MergeRequestInput, RequestLifecycleMutation, SubmitRequestInput, merge_request, submit_request,
};

pub const REQUEST_REF_PREFIX: &str = "refs/heads/";
pub fn canonical_request_ref(request_name: &str) -> String {
    format!("{REQUEST_REF_PREFIX}{request_name}")
}

pub(super) fn validate_required_id(label: &str, value: &str) -> Result<(), DomainError> {
    if value.trim().is_empty() {
        return Err(DomainError::invalid_input(format!("{label} is required")));
    }
    Ok(())
}

pub(super) fn advance_request_activity(request: &mut Request) -> Result<u64, DomainError> {
    request.activity_version = request
        .activity_version
        .checked_add(1)
        .ok_or_else(|| DomainError::conflict("request activity version overflow"))?;
    Ok(request.activity_version)
}

pub(super) fn open_request_mut<'a>(
    requests: &'a mut BTreeMap<String, Request>,
    request_id: &str,
) -> Result<&'a mut Request, DomainError> {
    let request = requests
        .get_mut(request_id)
        .ok_or_else(|| DomainError::not_found("request not found"))?;
    if request.is_terminal() {
        return Err(DomainError::conflict("request is closed"));
    }
    Ok(request)
}

pub(super) fn ensure_event_id_available(
    events: &BTreeMap<String, RequestEvent>,
    event_id: &str,
) -> Result<(), DomainError> {
    if events.contains_key(event_id) {
        Err(DomainError::conflict("request event already exists"))
    } else {
        Ok(())
    }
}
