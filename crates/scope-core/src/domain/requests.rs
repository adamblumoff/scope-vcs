use crate::error::ApiError;
use std::collections::BTreeMap;

mod access;
pub use access::{
    RequestMergeability, RequestMergeabilityStatus, RequestPermissions, request_actor_role,
    request_counts_as_open, request_is_published, request_list_mergeability, request_mergeability,
    request_permissions, request_visible_audiences, request_visible_to_access,
};
mod change_blocks;
pub use change_blocks::RequestChangeBlock;
mod credits;
pub use credits::{
    CreditAccountMutation, CreditLedgerEntry, CreditLedgerEntryKind, GrantUserCreditsInput,
    RequestSettlement, UserCreditAccount, grant_user_credits, settlement_for,
};
mod description;
pub use description::{UpdateRequestDescriptionInput, update_request_description};
mod discussions;
pub use discussions::{
    CreateRequestDiscussionInput, CreateRequestDiscussionMutation,
    CreateRequestDiscussionReplyInput, CreateRequestDiscussionReplyMutation,
    MarkRequestDiscussionReadInput, ReopenAndReplyToRequestDiscussionInput,
    ReopenRequestDiscussionInput, RequestDiscussion, RequestDiscussionMutation,
    RequestDiscussionReadState, RequestDiscussionReply, RequestDiscussionStatus,
    RequestDiscussionSubject, ResolveRequestDiscussionInput, create_request_discussion,
    create_request_discussion_reply, mark_request_discussion_read,
    reopen_and_reply_to_request_discussion, reopen_request_discussion, resolve_request_discussion,
};
mod lifecycle;
pub use lifecycle::{
    CloseRequestInput, CloseRequestMutation, RecordRequestRevisionInput,
    RecordWorkingRequestUploadInput, RequestRevisionMutation, StartRequestInput,
    StartRequestMutation, WorkingRequestUploadMutation, close_request, record_request_revision,
    record_working_request_upload, start_request, validate_request_name,
};
mod limits;
pub use limits::{
    REQUEST_ACTIVITY_PAGE_MAX_EVENTS, REQUEST_DESCRIPTION_MAX_BYTES,
    REQUEST_DISCUSSION_BODY_MAX_BYTES, REQUEST_DISCUSSION_CLIENT_ID_MAX_BYTES,
    REQUEST_DISCUSSION_REPLY_MAX_DEPTH, REQUEST_LIST_DEFAULT_PAGE_SIZE, REQUEST_LIST_MAX_PAGE_SIZE,
    REQUEST_TIMELINE_BODY_MAX_BYTES, REQUEST_TITLE_MAX_BYTES,
};
pub(super) use limits::{validate_body_size, validate_required_body};
mod model;
pub use model::{
    Request, RequestActorRole, RequestAudience, RequestDescriptionAuditFact, RequestEvent,
    RequestEventKind, RequestEventPayload, RequestInvitee, RequestState, RequestTimelineMutation,
    validate_request_facts,
};
mod review;
pub use review::{
    REQUEST_MAX_STAKE_CREDITS, RequestAssessmentOutcome, RequestReviewExitReason,
    validate_assessment_body,
};

pub const REQUEST_REF_PREFIX: &str = "refs/heads/";
pub fn canonical_request_ref(request_name: &str) -> String {
    format!("{REQUEST_REF_PREFIX}{request_name}")
}

pub(super) fn validate_required_id(label: &str, value: &str) -> Result<(), ApiError> {
    if value.trim().is_empty() {
        return Err(ApiError::bad_request(format!("{label} is required")));
    }
    Ok(())
}

pub(super) fn advance_request_activity(request: &mut Request) -> Result<u64, ApiError> {
    request.activity_version = request
        .activity_version
        .checked_add(1)
        .ok_or_else(|| ApiError::conflict("request activity version overflow"))?;
    Ok(request.activity_version)
}

pub(super) fn open_request_mut<'a>(
    requests: &'a mut BTreeMap<String, Request>,
    request_id: &str,
) -> Result<&'a mut Request, ApiError> {
    let request = requests
        .get_mut(request_id)
        .ok_or_else(|| ApiError::not_found("request not found"))?;
    if request.state == RequestState::Completed {
        return Err(ApiError::conflict("request is completed"));
    }
    Ok(request)
}

pub(super) fn ensure_event_id_available(
    events: &BTreeMap<String, RequestEvent>,
    event_id: &str,
) -> Result<(), ApiError> {
    if events.contains_key(event_id) {
        Err(ApiError::conflict("request event already exists"))
    } else {
        Ok(())
    }
}
