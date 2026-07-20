use super::{
    Request, RequestDiscussion, RequestDiscussionStatus, RequestDiscussionSubject, RequestEvent,
};
use crate::{domain::store::SourceBlob, error::ApiError};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RequestChangeBlock {
    pub id: String,
    pub request_id: String,
    pub position: u64,
    pub actor_user_id: String,
    pub old_head_oid: String,
    pub new_head_oid: String,
    pub git_snapshot: SourceBlob,
    pub created_at_unix: u64,
}

pub(super) fn submitted_change_block(
    request: &Request,
    event: &RequestEvent,
) -> Result<(RequestChangeBlock, RequestDiscussion), ApiError> {
    change_block(
        request,
        event,
        request.base_main_oid.clone(),
        request.head_oid.clone(),
    )
}

pub(super) fn revision_change_block(
    request: &Request,
    event: &RequestEvent,
    old_head_oid: String,
    new_head_oid: String,
) -> Result<(RequestChangeBlock, RequestDiscussion), ApiError> {
    change_block(request, event, old_head_oid, new_head_oid)
}

fn change_block(
    request: &Request,
    event: &RequestEvent,
    old_head_oid: String,
    new_head_oid: String,
) -> Result<(RequestChangeBlock, RequestDiscussion), ApiError> {
    let git_snapshot = request
        .git_snapshot
        .clone()
        .ok_or_else(|| ApiError::conflict("request change block requires an uploaded snapshot"))?;
    let block = RequestChangeBlock {
        id: event.id.clone(),
        request_id: request.id.clone(),
        position: event.position,
        actor_user_id: event.actor_user_id.clone(),
        old_head_oid,
        new_head_oid,
        git_snapshot,
        created_at_unix: event.created_at_unix,
    };
    let discussion = RequestDiscussion {
        id: format!("thread_{}", block.id),
        request_id: request.id.clone(),
        opened_position: block.position,
        last_activity_position: block.position,
        author_user_id: block.actor_user_id.clone(),
        subject: RequestDiscussionSubject::ChangeBlock {
            change_block_id: block.id.clone(),
        },
        body_markdown: None,
        status: RequestDiscussionStatus::Dormant,
        client_discussion_id: format!("change-block:{}", block.id),
        created_at_unix: block.created_at_unix,
        resolved_at_unix: None,
        resolved_by_user_id: None,
    };
    Ok((block, discussion))
}
